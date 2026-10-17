// Original project-owned virtual filespace. No USVFS implementation code is used.
#include "filespace.hpp"
#include <MinHook.h>
#include <bcrypt.h>
#include <algorithm>
#include <cstddef>
#include <cstring>
#include <map>
#include <memory>
#include <mutex>
#include <set>
#include <vector>

constexpr NTSTATUS ok = 0, denied = static_cast<NTSTATUS>(0xC0000022), unsupported = static_cast<NTSTATUS>(0xC00000BB);
constexpr NTSTATUS missing = static_cast<NTSTATUS>(0xC0000034), collision = static_cast<NTSTATUS>(0xC0000035);
constexpr NTSTATUS no_more = static_cast<NTSTATUS>(0x80000006), too_small = static_cast<NTSTATUS>(0xC0000023);
constexpr ULONG write_rights = FILE_WRITE_DATA | FILE_APPEND_DATA | FILE_WRITE_ATTRIBUTES | FILE_WRITE_EA | DELETE | WRITE_DAC | WRITE_OWNER | GENERIC_WRITE | GENERIC_ALL;
struct Ordinal {
    bool operator()(const std::wstring& a, const std::wstring& b) const {
        return CompareStringOrdinal(a.c_str(), static_cast<int>(a.size()), b.c_str(), static_cast<int>(b.size()), TRUE) == CSTR_LESS_THAN;
    }
};
using Paths = std::map<std::wstring, std::wstring, Ordinal>;
static Paths files, mounts;
static std::set<std::wstring, Ordinal> virtual_dirs;
static std::set<std::wstring, Ordinal> protected_roots;
static std::map<std::wstring, std::set<std::wstring, Ordinal>, Ordinal> children;
static std::wstring owner, session;
static thread_local unsigned bypass = 0, spawning = 0;
static bool initialized = false;
#ifdef MODBOX_VFS_TESTING
static ULONG test_mode=0;static std::wstring test_events;static bool test_publish_failure=false;
extern "C" __declspec(dllexport) void WINAPI MBVFS_TestControl(ULONG mode,const wchar_t* events) {test_mode=mode;test_events=events?events:L"";}
static void mb_test_after_rename() {
    if(test_mode==1){test_publish_failure=true;test_mode=0;return;}
    if(test_mode==2) {
        HANDLE entered=OpenEventW(EVENT_MODIFY_STATE,FALSE,(test_events+L"_entered").c_str());
        HANDLE release=OpenEventW(SYNCHRONIZE,FALSE,(test_events+L"_release").c_str());
        if(entered&&release){SetEvent(entered);WaitForSingleObject(release,30000);}
        if(entered)CloseHandle(entered);if(release)CloseHandle(release);test_mode=0;
    }
}
static void mb_test_after_delete_state() {if(test_mode==3){test_mode=2;mb_test_after_rename();}}
#endif
struct Guard { Guard() { ++bypass; } ~Guard() { --bypass; } };
struct Entry { std::wstring name; WIN32_FILE_ATTRIBUTE_DATA data{}; };
struct Opened { std::wstring path, output; ULONGLONG lease=0; ACCESS_MASK access=0; bool directory = false, inheritable=false; std::vector<Entry> entries; size_t cursor = 0; bool scanned = false; std::wstring pattern; };
static std::recursive_mutex handles_mutex;
static std::map<HANDLE, std::shared_ptr<Opened>> handles;

using CreateFn = NTSTATUS(NTAPI*)(PHANDLE, ACCESS_MASK, POBJECT_ATTRIBUTES, PIO_STATUS_BLOCK, PLARGE_INTEGER, ULONG, ULONG, ULONG, ULONG, PVOID, ULONG);
using OpenFn = NTSTATUS(NTAPI*)(PHANDLE, ACCESS_MASK, POBJECT_ATTRIBUTES, PIO_STATUS_BLOCK, ULONG, ULONG);
using QueryFn = NTSTATUS(NTAPI*)(POBJECT_ATTRIBUTES, PVOID);
using DirFn = NTSTATUS(NTAPI*)(HANDLE, HANDLE, PIO_APC_ROUTINE, PVOID, PIO_STATUS_BLOCK, PVOID, ULONG, FILE_INFORMATION_CLASS, BOOLEAN, PUNICODE_STRING, BOOLEAN);
using DirExFn = NTSTATUS(NTAPI*)(HANDLE, HANDLE, PIO_APC_ROUTINE, PVOID, PIO_STATUS_BLOCK, PVOID, ULONG, FILE_INFORMATION_CLASS, ULONG, PUNICODE_STRING);
using SetFn = NTSTATUS(NTAPI*)(HANDLE, PIO_STATUS_BLOCK, PVOID, ULONG, FILE_INFORMATION_CLASS);
using DeleteFn = NTSTATUS(NTAPI*)(POBJECT_ATTRIBUTES);
using CloseFn = NTSTATUS(NTAPI*)(HANDLE);
using DuplicateFn = NTSTATUS(NTAPI*)(HANDLE, HANDLE, HANDLE, PHANDLE, ACCESS_MASK, ULONG, ULONG);
using UserProcessFn = NTSTATUS(NTAPI*)(PHANDLE, PHANDLE, ACCESS_MASK, ACCESS_MASK, POBJECT_ATTRIBUTES, POBJECT_ATTRIBUTES, ULONG, ULONG, PVOID, PVOID, PVOID);
static CreateFn real_create = nullptr;
static OpenFn real_open = nullptr;
static QueryFn real_attributes = nullptr, real_full_attributes = nullptr;
static DirFn real_dir = nullptr;
static DirExFn real_dir_ex = nullptr;
static SetFn real_set = nullptr;
static DeleteFn real_delete = nullptr;
static CloseFn real_close = nullptr;
static DuplicateFn real_duplicate = nullptr;
static UserProcessFn real_user_process = nullptr;
static decltype(&CreateProcessW) real_process_w = nullptr;
static decltype(&CreateProcessA) real_process_a = nullptr;
static decltype(&FindWindowA) real_find_window_a = nullptr;
static decltype(&FindWindowW) real_find_window_w = nullptr;
static bool game_window_scope = false;

static bool fallout_image(const wchar_t* path) {
    const wchar_t* name = wcsrchr(path,L'\\');
    return _wcsicmp(name ? name+1 : path,L"Fallout4.exe") == 0;
}
static bool foreign_game_window(HWND window) {
    DWORD pid = 0; GetWindowThreadProcessId(window,&pid);
    if (!pid || pid == GetCurrentProcessId()) return false;
    HANDLE process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,FALSE,pid);
    if (!process) return false;
    std::vector<wchar_t> path(32768); DWORD size = static_cast<DWORD>(path.size());
    const bool foreign = QueryFullProcessImageNameW(process,0,path.data(),&size) && fallout_image(path.data());
    CloseHandle(process);
    return foreign;
}
// Main::StartUp (AE 1.11.240 RVA 0xc2fff0) exits on a foreign
// FindWindowA result. Keep later own-window lookups working as well.
static HWND WINAPI hook_find_window_a(LPCSTR type,LPCSTR title) {
    HWND window = real_find_window_a(type,title);
    while (game_window_scope && window && foreign_game_window(window))
        window = FindWindowExA(nullptr,window,type,title);
    return window;
}
static HWND WINAPI hook_find_window_w(LPCWSTR type,LPCWSTR title) {
    HWND window = real_find_window_w(type,title);
    while (game_window_scope && window && foreign_game_window(window))
        window = FindWindowExW(nullptr,window,type,title);
    return window;
}

static bool equal(const std::wstring& a, const std::wstring& b) { return !Ordinal{}(a,b) && !Ordinal{}(b,a); }
static bool within(const std::wstring& path, const std::wstring& root) {
    return path.size() >= root.size() && equal(path.substr(0, root.size()), root) && (path.size() == root.size() || path[root.size()] == L'\\');
}
static std::wstring parent(const std::wstring& path) { size_t pos = path.rfind(L'\\'); return pos == std::wstring::npos ? L"" : path.substr(0, pos); }
static bool protected_path(std::wstring path) {
    while (path.size() >= 3) { if (protected_roots.contains(path)) return true; path = parent(path); }
    return false;
}
static bool owned_backing(const std::wstring& path) {
    if(within(path,owner+L"\\.filespace-state")||within(path,owner+L"\\.filespace-cow"))return true;
    for(auto& [target,source]:mounts){(void)target;if(within(path,source))return true;}
    for(auto& [target,source]:files){(void)target;if(equal(path,source)&&within(source,owner+L"\\output"))return true;}
    return false;
}
static std::wstring folded(std::wstring value) {
    if (value.empty()) return value;
    int count = LCMapStringEx(LOCALE_NAME_INVARIANT,LCMAP_UPPERCASE,value.c_str(),static_cast<int>(value.size()),nullptr,0,nullptr,nullptr,0);
    std::wstring result(count,L'\0');
    if (!count || !LCMapStringEx(LOCALE_NAME_INVARIANT,LCMAP_UPPERCASE,value.c_str(),static_cast<int>(value.size()),result.data(),count,nullptr,nullptr,0)) return value;
    return result;
}
static std::wstring normalize(std::wstring path) {
    std::replace(path.begin(), path.end(), L'/', L'\\');
    if (path.starts_with(L"\\??\\") || path.starts_with(L"\\\\?\\")) path.erase(0, 4);
    if (path.starts_with(L"\\Device\\")) {
        std::vector<wchar_t> target(32768);
        for (wchar_t drive = L'A'; drive <= L'Z'; ++drive) {
            wchar_t name[] = {drive, L':', 0};
            if (QueryDosDeviceW(name, target.data(), static_cast<DWORD>(target.size())) && within(path, target.data())) { path = std::wstring(name) + path.substr(wcslen(target.data())); break; }
        }
    }
    if (path.size() < 3 || path[1] != L':') return L"";  // UNC/devices are outside this local backend.
    std::vector<wchar_t> buffer(32768);
    DWORD length = GetFullPathNameW(path.c_str(), static_cast<DWORD>(buffer.size()), buffer.data(), nullptr);
    if (!length || length >= buffer.size()) return L"";
    path.assign(buffer.data(), length);
    while (path.size() > 3 && path.back() == L'\\') path.pop_back();
    return path;
}
static std::wstring object_path(POBJECT_ATTRIBUTES attr) {
    if (!attr || !attr->ObjectName || attr->ObjectName->Length % 2) return L"";
    std::wstring path(attr->ObjectName->Buffer, attr->ObjectName->Length / 2);
    if (attr->RootDirectory) {
        std::lock_guard<std::recursive_mutex> lock(handles_mutex);
        auto item = handles.find(attr->RootDirectory);
        if (item != handles.end()) path = item->second->path + L"\\" + path;
        else {
            std::vector<wchar_t> name(32768);
            DWORD count = GetFinalPathNameByHandleW(attr->RootDirectory, name.data(), static_cast<DWORD>(name.size()), FILE_NAME_NORMALIZED | VOLUME_NAME_DOS);
            if (!count || count >= name.size()) return L"";
            path = std::wstring(name.data()) + L"\\" + path;
        }
    }
    return normalize(path);
}
struct Object {
    std::wstring name; UNICODE_STRING text{}; OBJECT_ATTRIBUTES attributes{};
    Object(const std::wstring& path, POBJECT_ATTRIBUTES original) : name(L"\\??\\" + path), attributes(*original) {
        text.Length = static_cast<USHORT>(name.size() * sizeof(wchar_t)); text.MaximumLength = text.Length;
        text.Buffer = name.data(); attributes.RootDirectory = nullptr; attributes.ObjectName = &text;
    }
};
static bool safe_path(const std::wstring& path, bool output = false) {
    if (path.empty() || path.size() > 32000 || path.find(L':', 2) != std::wstring::npos) return false;
    for (size_t pos = 3;;) {
        pos = path.find(L'\\', pos);
        std::wstring current = pos == std::wstring::npos ? path : path.substr(0, pos);
        DWORD attr = GetFileAttributesW(current.c_str());
        if (attr != INVALID_FILE_ATTRIBUTES && (attr & FILE_ATTRIBUTE_REPARSE_POINT)) return false;
        if (pos == std::wstring::npos) break;
        ++pos;
    }
    if (output && GetFileAttributesW(path.c_str()) != INVALID_FILE_ATTRIBUTES) {
        HANDLE h = CreateFileW(path.c_str(), FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
        if (h == INVALID_HANDLE_VALUE) return false;
        BY_HANDLE_FILE_INFORMATION info{};
        bool safe = GetFileInformationByHandle(h, &info) && info.nNumberOfLinks <= 1;
        CloseHandle(h);
        if (!safe) return false;
    }
    return true;
}
// Bounded snapshot operations can share ancestor checks. Actual intercepted
// opens/writes still use fresh safe_path checks; no cache outlives one snapshot.
static bool safe_snapshot_path(const std::wstring& path, std::set<std::wstring,Ordinal>& checked) {
    if (path.empty() || path.size() > 32000 || path.find(L':',2) != std::wstring::npos) return false;
    for (size_t pos=3;;) {
        pos = path.find(L'\\',pos);
        auto current = pos == std::wstring::npos ? path : path.substr(0,pos);
        if (!checked.contains(current)) {
            DWORD attributes = GetFileAttributesW(current.c_str());
            if (attributes != INVALID_FILE_ATTRIBUTES && (attributes & FILE_ATTRIBUTE_REPARSE_POINT)) return false;
            checked.emplace(std::move(current));
        }
        if (pos == std::wstring::npos) break;
        ++pos;
    }
    return true;
}
static bool mkdirs(const std::wstring& path) {
    if (!within(path, owner) || !safe_path(path, true)) return false;
    for (size_t pos = 3;;) {
        pos = path.find(L'\\', pos);
        std::wstring current = pos == std::wstring::npos ? path : path.substr(0, pos);
        DWORD attr = GetFileAttributesW(current.c_str());
        if (attr == INVALID_FILE_ATTRIBUTES && !CreateDirectoryW(current.c_str(), nullptr) && GetLastError() != ERROR_ALREADY_EXISTS) return false;
        if (pos == std::wstring::npos) break;
        ++pos;
    }
    return safe_path(path, true);
}
#include "overlay_state.inc"
struct Selection { bool managed = false, exists = false, directory = false; std::wstring physical, output; };
static Selection select(const std::wstring& path) {
    Selection result;
    std::wstring root;
    for (const auto& [target, source] : mounts) if (within(path, target) && target.size() > root.size()) { root = target; result.output = source + path.substr(target.size()); }
    auto file = files.find(path);
    // Exact owned runtime snapshots (INIs/plugin state) are their own writable
    // backing. Other source maps still require an output mount for writes.
    // This holds under any enclosing mount too (game-root output, Data overwrite).
    if (file != files.end() && within(file->second, owner+L"\\output")) result.output = file->second;
    result.managed = !root.empty() || file != files.end();
    if (!result.managed) return result;
    if(!result.output.empty()&&hidden(result.output))return result;
    bool lower = result.output.empty()||!lower_hidden(result.output);
    if (!result.output.empty() && GetFileAttributesW(result.output.c_str()) != INVALID_FILE_ATTRIBUTES) result.physical = result.output;
    else if (lower && file != files.end()) result.physical = file->second;
    else if (lower && GetFileAttributesW(path.c_str()) != INVALID_FILE_ATTRIBUTES) result.physical = path;
    else if (lower && (virtual_dirs.contains(path) || equal(path, root))) {
        if (!result.output.empty() && mkdirs(result.output)) result.physical = result.output;
    }
    if (!result.physical.empty()) {
        DWORD attr = GetFileAttributesW(result.physical.c_str());
        result.exists = attr != INVALID_FILE_ATTRIBUTES;
        result.directory = result.exists && (attr & FILE_ATTRIBUTE_DIRECTORY);
    }
    return result;
}
static NTSTATUS failure(PIO_STATUS_BLOCK io, NTSTATUS status) { if (io) { io->Status = status; io->Information = 0; } return status; }
static DWORD share_access(ACCESS_MASK access) {
    DWORD result=0;
    if(access&(FILE_READ_DATA|FILE_EXECUTE|GENERIC_READ|GENERIC_EXECUTE|GENERIC_ALL))result|=FILE_SHARE_READ;
    if(access&(FILE_WRITE_DATA|FILE_APPEND_DATA|GENERIC_WRITE|GENERIC_ALL))result|=FILE_SHARE_WRITE;
    if(access&(DELETE|GENERIC_ALL))result|=FILE_SHARE_DELETE;
    return result;
}
static NTSTATUS mutate(HANDLE h,PIO_STATUS_BLOCK io,PVOID buffer,ULONG length,ULONG cls);
static bool remove_hidden_output(const std::wstring& output,bool root);
static NTSTATUS create(PHANDLE handle, ACCESS_MASK access, POBJECT_ATTRIBUTES attr, PIO_STATUS_BLOCK io, PLARGE_INTEGER allocation,
        ULONG attributes, ULONG share, ULONG disposition, ULONG options, PVOID ea, ULONG ea_size) {
    std::wstring path = object_path(attr);
    Selection item = select(path);
    bool write = (access & write_rights) || disposition != 1 || (options & 0x1000); // FILE_DELETE_ON_CLOSE
    if (!item.managed) {
        if (write && (owned_backing(path)||protected_path(path))) return failure(io, denied);
        return real_create(handle, access, attr, io, allocation, attributes, share, disposition, options, ea, ea_size);
    }
    if ((options & 0x2000) || ((options & 1) && (attr->Attributes & OBJ_INHERIT)) || !safe_path(path)) return failure(io, unsupported);
    if(item.output.size()>=lease_path_capacity)return failure(io,unsupported);
    bool free_lease=false;for(auto& lease:shared_overlay->leases)if(!live_lease(lease)){free_lease=true;break;}
    if(!free_lease)return failure(io,unsupported);
    if(auto pending=pending_deletes.find(item.output);pending!=pending_deletes.end()&&generation_open(item.output,pending->second))return failure(io,static_cast<NTSTATUS>(0xC0000056));
    if (disposition == 2 && item.exists) return failure(io, collision); // CREATE_NEW tests merged existence.
    if ((disposition == 1 || disposition == 4) && !item.exists) return failure(io, missing);
    if (write) {
        if (item.output.empty()) return failure(io, denied);
        auto containing=select(parent(path));
        if(containing.managed&&(!containing.exists||!containing.directory))return failure(io,missing);
        if (!mkdirs(parent(item.output)) || !safe_path(item.output, true)) return failure(io, denied);
        if(item.directory&&!mkdirs(item.output))return failure(io,denied);
        if(item.exists&&!sharing_allowed(item.output,identity(item.physical),share_access(access),share))return failure(io,static_cast<NTSTATUS>(0xC0000043));
        // Cross-process first-write serialization. Unmodified files remain referenced in place.
        bool copied = true;
        if (item.exists && !item.directory && !equal(item.physical, item.output) && GetFileAttributesW(item.output.c_str()) == INVALID_FILE_ATTRIBUTES) {
            auto stage = owner + L"\\.filespace-cow";
            auto temporary = stage + L"\\" + session + L"_" + std::to_wstring(GetCurrentProcessId()) + L"_" + std::to_wstring(GetCurrentThreadId()) + L"_" + std::to_wstring(GetTickCount64());
            copied = mkdirs(stage) && safe_path(item.physical) && CopyFileW(item.physical.c_str(), temporary.c_str(), TRUE);
            if (copied) copied = safe_path(item.output,true) && MoveFileExW(temporary.c_str(),item.output.c_str(),MOVEFILE_WRITE_THROUGH);
            DeleteFileW(temporary.c_str()); // Interrupted copies stay outside every virtual mount.
        }
        if (!copied || !safe_path(item.output, true)) return failure(io, denied);
        item.physical = item.output;
    }
    if (item.physical.empty()) return failure(io, missing);
    if (!safe_path(item.physical, write)) return failure(io, denied);
    Object target(item.physical, attr);
    bool recreate=whiteouts.contains(item.output);
    auto before=recreate?encode_state():std::vector<char>{};
    if(recreate) {
        // A committed deletion may retain an owned backing after a crash. Never
        // reuse that object or resurrect its descendants when creating anew.
        auto old=identity(item.output);
        if(old&&generation_open(item.output,old))return failure(io,static_cast<NTSTATUS>(0xC0000056));
        DWORD prior=GetFileAttributesW(item.output.c_str());
        if(prior!=INVALID_FILE_ATTRIBUTES&&!remove_hidden_output(item.output,true))return failure(io,denied);
    }
    NTSTATUS status = real_create(handle, access, &target.attributes, io, allocation, attributes, share, disposition, options&~0x1000u, ea, ea_size);
    if (status >= 0 && handle) {
        std::lock_guard<std::recursive_mutex> lock(handles_mutex);
        auto opened = std::make_shared<Opened>(); opened->path = path; opened->output=item.output;opened->access=access;opened->inheritable=(attr->Attributes&OBJ_INHERIT)!=0; opened->directory = item.directory || (options & 1);
        opened->lease=add_lease(item.output.empty()?path:item.output,*handle,share_access(access),share,opened->directory);
        if(!opened->lease){real_close(*handle);return failure(io,unsupported);}
        handles[*handle] = opened;
        if(recreate) {
            whiteouts.erase(item.output);pending_deletes.erase(item.output);if(opened->directory)opaque_dirs.insert(item.output);
            if(!publish_state()) {decode_state(before);handles.erase(*handle);drop_lease(opened->lease);real_close(*handle);if(opened->directory)RemoveDirectoryW(item.output.c_str());else DeleteFileW(item.output.c_str());return failure(io,denied);}
        }
        if(options&0x1000){BOOLEAN remove=TRUE;status=mutate(*handle,io,&remove,1,13);if(status<0){handles.erase(*handle);drop_lease(opened->lease);real_close(*handle);}}
    }
    return status;
}
static NTSTATUS NTAPI hook_create(PHANDLE h, ACCESS_MASK a, POBJECT_ATTRIBUTES o, PIO_STATUS_BLOCK i, PLARGE_INTEGER z, ULONG f, ULONG s, ULONG d, ULONG c, PVOID e, ULONG n) {
    if (bypass || !initialized) return real_create(h,a,o,i,z,f,s,d,c,e,n);
    Guard guard;
    try { NamespaceLock lock;if(!lock.valid)return failure(i,denied);return create(h,a,o,i,z,f,s,d,c,e,n); } catch (...) { return failure(i, denied); }
}
static NTSTATUS NTAPI hook_open(PHANDLE h, ACCESS_MASK a, POBJECT_ATTRIBUTES o, PIO_STATUS_BLOCK i, ULONG s, ULONG c) {
    if (bypass || !initialized) return real_open(h,a,o,i,s,c);
    Guard guard;
    try { NamespaceLock lock;if(!lock.valid)return failure(i,denied);return create(h,a,o,i,nullptr,0,s,1,c,nullptr,0); } catch (...) { return failure(i, denied); }
}
static NTSTATUS attributes(POBJECT_ATTRIBUTES attr, PVOID data, QueryFn original) {
    if (bypass || !initialized) return original(attr, data);
    Guard guard;
    try {
        NamespaceLock lock;if(!lock.valid)return denied;
        auto path = object_path(attr); auto item = select(path);
        if (!item.managed) return original(attr, data);
        if (!item.exists) return missing;
        if (!safe_path(item.physical)) return denied;
        Object target(item.physical, attr);
        return original(&target.attributes, data);
    } catch (...) { return denied; }
}
static NTSTATUS NTAPI hook_attributes(POBJECT_ATTRIBUTES o, PVOID d) { return attributes(o,d,real_attributes); }
static NTSTATUS NTAPI hook_full_attributes(POBJECT_ATTRIBUTES o, PVOID d) { return attributes(o,d,real_full_attributes); }

// FsRtlIsNameInExpression semantics, including the DOS forms FindFirstFileW sends:
// '<' (DOS_STAR) stops before the final '.', '>' (DOS_QM) and '"' (DOS_DOT) may match nothing at a '.'/end.
static bool dos_wildcard(const wchar_t* pattern, const wchar_t* name, const wchar_t* last_dot, int depth) {
    if (depth > 64) return false;
    for (;;) {
        wchar_t c = *pattern;
        if (!c) return !*name;
        if (c == L'*' || c == L'<') {
            for (const wchar_t* next = name;; ++next) {
                if (dos_wildcard(pattern + 1, next, last_dot, depth + 1)) return true;
                if (!*next || (c == L'<' && last_dot && next >= last_dot)) return false;
            }
        }
        if (c == L'>') { if (*name && *name != L'.') ++name; ++pattern; continue; }
        if (c == L'"') { if (*name == L'.') ++name; else if (*name) return false; ++pattern; continue; }
        if (!*name || (c != L'?' && CompareStringOrdinal(pattern, 1, name, 1, TRUE) != CSTR_EQUAL)) return false;
        ++pattern; ++name;
    }
}
static bool wildcard(const wchar_t* pattern, const wchar_t* name) {
    if (!wcscmp(pattern, L"*.*")) pattern = L"*";
    return dos_wildcard(pattern, name, wcsrchr(name, L'.'), 0);
}
static void physical_names(const std::wstring& path, std::set<std::wstring, Ordinal>& names) {
    WIN32_FIND_DATAW data{};
    HANDLE found = FindFirstFileW((path + L"\\*").c_str(), &data);
    if (found == INVALID_HANDLE_VALUE) return;
    do { if (wcscmp(data.cFileName,L".") && wcscmp(data.cFileName,L"..")) names.emplace(data.cFileName); } while (FindNextFileW(found, &data));
    FindClose(found);
}
static std::vector<Entry> enumeration(const std::wstring& path, const std::wstring& pattern) {
    std::set<std::wstring, Ordinal> names;
    physical_names(path, names);
    auto dir = select(path);
    if (!dir.output.empty()) physical_names(dir.output, names);
    if (auto found = children.find(path); found != children.end()) names.insert(found->second.begin(),found->second.end());
    if (names.size() > 200000) throw std::runtime_error("Directory inventory exceeds bound");
    std::vector<Entry> result;
    // One directory query is a snapshot. Reused backing files need one safety/
    // metadata query, not another ancestor walk for every virtual alias.
    std::map<std::wstring, WIN32_FILE_ATTRIBUTE_DATA, Ordinal> metadata;
    std::set<std::wstring,Ordinal> checked;
    for (const auto& name : names) {
        if (!wildcard(pattern.c_str(), name.c_str())) continue;
        auto selected = select(path + L"\\" + name);
        Entry entry{name};
        auto physical = selected.managed ? selected.physical : path + L"\\" + name;
        if (physical.empty()) continue;
        auto found = metadata.find(physical);
        if (found != metadata.end()) entry.data = found->second;
        else {
            if (!safe_snapshot_path(physical,checked) || !GetFileAttributesExW(physical.c_str(), GetFileExInfoStandard, &entry.data)
                || (entry.data.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT)) continue;
            metadata.emplace(physical, entry.data);
        }
        result.push_back(entry);
    }
    return result;
}
static void store(void* buffer, size_t offset, const void* value, size_t bytes) { memcpy(static_cast<char*>(buffer) + offset, value, bytes); }
static NTSTATUS directory(HANDLE h, HANDLE event, PIO_APC_ROUTINE apc, PIO_STATUS_BLOCK io, PVOID buffer, ULONG length, FILE_INFORMATION_CLASS cls,
        bool single, PUNICODE_STRING filter, bool restart) {
    std::lock_guard<std::recursive_mutex> lock(handles_mutex);
    auto found = handles.find(h);
    if (found == handles.end() || !found->second->directory) return unsupported;
    if (apc || !io || !buffer) return failure(io, unsupported);
    ULONG kind = static_cast<ULONG>(cls), base = kind == 1 ? 64 : kind == 2 ? 68 : kind == 3 ? 94 : kind == 12 ? 12 : kind == 37 ? 104 : kind == 38 ? 80 : 0;
    if (!base) return failure(io, unsupported);
    auto& item = *found->second;
    if (!item.scanned || restart) {
        if (filter && !item.scanned) item.pattern.assign(filter->Buffer, filter->Length / 2);
        if (item.pattern.empty()) item.pattern = L"*";
        item.entries = enumeration(item.path, item.pattern); item.cursor = 0; item.scanned = true;
    }
    if (item.cursor >= item.entries.size()) return failure(io, no_more);
    size_t used = 0, previous = 0;
    while (item.cursor < item.entries.size()) {
        const auto& entry = item.entries[item.cursor];
        ULONG bytes = static_cast<ULONG>(entry.name.size() * 2), raw = base + bytes, size = (raw + 7) & ~7u;
        if (used + raw > length) { if (!used) return failure(io, too_small); break; }
        auto target = static_cast<char*>(buffer) + used;
        memset(target, 0, raw);
        store(target, kind == 12 ? 8 : 60, &bytes, 4);
        store(target, base, entry.name.data(), bytes);
        if (kind != 12) {
            store(target, 8, &entry.data.ftCreationTime, 8); store(target, 16, &entry.data.ftLastAccessTime, 8);
            store(target, 24, &entry.data.ftLastWriteTime, 8); store(target, 32, &entry.data.ftLastWriteTime, 8);
            ULONGLONG size64 = (static_cast<ULONGLONG>(entry.data.nFileSizeHigh) << 32) | entry.data.nFileSizeLow;
            store(target, 40, &size64, 8); store(target, 48, &size64, 8); store(target, 56, &entry.data.dwFileAttributes, 4);
            if (kind == 37 || kind == 38) { ULONGLONG id = std::hash<std::wstring>{}(item.path + L"\\" + entry.name); store(target, kind == 37 ? 96 : 72, &id, 8); }
        }
        if (used) { ULONG next = static_cast<ULONG>(used - previous); store(buffer, previous, &next, 4); }
        previous = used; used += raw; ++item.cursor;
        if (single || used + (size - raw) > length) break;
        used += size - raw;
    }
    io->Status = ok; io->Information = used;
    if (event) SetEvent(event);
    return ok;
}
static bool managed_handle(HANDLE h) { std::lock_guard<std::recursive_mutex> lock(handles_mutex); return handles.contains(h); }
static NTSTATUS NTAPI hook_dir(HANDLE h,HANDLE e,PIO_APC_ROUTINE a,PVOID c,PIO_STATUS_BLOCK i,PVOID b,ULONG l,FILE_INFORMATION_CLASS f,BOOLEAN s,PUNICODE_STRING n,BOOLEAN r) {
    if (bypass || !initialized || !managed_handle(h)) return real_dir(h,e,a,c,i,b,l,f,s,n,r);
    Guard guard; try { NamespaceLock lock;if(!lock.valid)return failure(i,denied);return directory(h,e,a,i,b,l,f,s != FALSE,n,r != FALSE); } catch (...) { return failure(i, denied); }
}
static NTSTATUS NTAPI hook_dir_ex(HANDLE h,HANDLE e,PIO_APC_ROUTINE a,PVOID c,PIO_STATUS_BLOCK i,PVOID b,ULONG l,FILE_INFORMATION_CLASS f,ULONG flags,PUNICODE_STRING n) {
    if (bypass || !initialized || !managed_handle(h)) return real_dir_ex(h,e,a,c,i,b,l,f,flags,n);
    Guard guard;
    if (flags & ~3u) return failure(i, unsupported);
    try { NamespaceLock lock;if(!lock.valid)return failure(i,denied);return directory(h,e,a,i,b,l,f,(flags & 2) != 0,n,(flags & 1) != 0); } catch (...) { return failure(i, denied); }
}
#include "mutations.inc"
static NTSTATUS NTAPI hook_set(HANDLE h,PIO_STATUS_BLOCK i,PVOID b,ULONG l,FILE_INFORMATION_CLASS f) {
    if (!bypass && initialized) {
        ULONG cls = static_cast<ULONG>(f);
        bool rename_link = cls == 10 || cls == 11 || cls == 56 || cls == 57 || cls == 65 || cls == 66 || cls == 72 || cls == 73;
        if (managed_handle(h) && (rename_link || cls == 13 || cls == 64)) {
            Guard guard;try{NamespaceLock lock;if(!lock.valid)return failure(i,denied);return mutate(h,i,b,l,cls);}catch(...){return failure(i,denied);}
        }
        if (rename_link) {
            // An unmanaged temporary handle must not rename/link into the physical
            // game or source namespace. Every path-bearing destination is checked.
            Guard guard;
            NamespaceLock lock;if(!lock.valid)return failure(i,denied);
            if (!b || l < 20) return failure(i, unsupported);
            ULONG bytes = 0; HANDLE root = nullptr;
            memcpy(&root,static_cast<char*>(b)+8,sizeof(root)); memcpy(&bytes,static_cast<char*>(b)+16,4);
            if (bytes > l-20 || bytes > 64000 || bytes % 2) return failure(i, unsupported);
            UNICODE_STRING name{}; name.Length = static_cast<USHORT>(bytes); name.MaximumLength = name.Length;
            name.Buffer = reinterpret_cast<wchar_t*>(static_cast<char*>(b)+20);
            OBJECT_ATTRIBUTES attrs{}; attrs.Length = sizeof(attrs); attrs.RootDirectory = root; attrs.ObjectName = &name;
            auto path = object_path(&attrs);
            if (path.empty() || select(path).managed || protected_path(path)||owned_backing(path)) return failure(i, unsupported);
        }
    }
    return real_set(h,i,b,l,f);
}
static NTSTATUS NTAPI hook_delete(POBJECT_ATTRIBUTES o) {
    if (bypass || !initialized) return real_delete(o);
    Guard guard;
    try {
        NamespaceLock lock;if(!lock.valid)return denied;
        auto path=object_path(o);auto item=select(path);
        if(item.managed) {
            HANDLE handle=nullptr;IO_STATUS_BLOCK io{};
            auto status=create(&handle,DELETE|SYNCHRONIZE,o,&io,nullptr,0,7,1,0x20,nullptr,0);
            if(status<0)return status;
            BOOLEAN remove=TRUE;status=mutate(handle,&io,&remove,1,13);
            auto found=handles.find(handle);if(found!=handles.end()){drop_lease(found->second->lease);handles.erase(found);}real_close(handle);return status;
        }
        if(protected_path(path)||owned_backing(path))return denied;
    }
    catch (...) { return denied; }
    return real_delete(o);
}
static NTSTATUS NTAPI hook_close(HANDLE h) {
    if(bypass||!initialized||!managed_handle(h))return real_close(h);
    Guard guard;try {
        NamespaceLock namespace_lock;if(!namespace_lock.valid)return denied;
        std::lock_guard<std::recursive_mutex> lock(handles_mutex);auto found=handles.find(h);if(found==handles.end())return real_close(h);
        auto item=found->second;NTSTATUS status=real_close(h);
        if(status>=0){handles.erase(found);if(item.use_count()==1)drop_lease(item->lease);}
        return status;
    }catch(...){return denied;}
}
static NTSTATUS NTAPI hook_duplicate(HANDLE sp,HANDLE h,HANDLE tp,PHANDLE out,ACCESS_MASK a,ULONG attrs,ULONG flags) {
    if (bypass || !initialized) return real_duplicate(sp,h,tp,out,a,attrs,flags);
    bool source_here=sp==GetCurrentProcess()||GetProcessId(sp)==GetCurrentProcessId();
    bool target_here=tp==GetCurrentProcess()||GetProcessId(tp)==GetCurrentProcessId();
    // Foreign handle numbers cannot be identified from this process's map.
    // Refuse foreign-source duplication and exporting disk handles; unmanaged
    // pipes/process handles can still support ordinary captured child output.
    if(!source_here||(!target_here&&GetFileType(h)==FILE_TYPE_DISK))return unsupported;
    if(!managed_handle(h))return real_duplicate(sp,h,tp,out,a,attrs,flags);
    if (!target_here || (attrs & OBJ_INHERIT) || (flags&~3u)) return unsupported;
    Guard guard;NamespaceLock namespace_lock;if(!namespace_lock.valid)return denied;
    std::lock_guard<std::recursive_mutex> lock(handles_mutex);auto original=handles[h];
    NTSTATUS status = real_duplicate(sp,h,tp,out,a,attrs,flags);
    if (status >= 0 && out)handles[*out]=original;
    // DUPLICATE_CLOSE_SOURCE closes the source even on a failed duplication.
    if(flags&1){handles.erase(h);if(original.use_count()==1)drop_lease(original->lease);}
    return status;
}
static NTSTATUS NTAPI hook_user_process(PHANDLE p,PHANDLE t,ACCESS_MASK pa,ACCESS_MASK ta,POBJECT_ATTRIBUTES po,POBJECT_ATTRIBUTES to,ULONG pf,ULONG tf,PVOID params,PVOID info,PVOID attrs) {
    if (initialized && !spawning) return unsupported; // Native/broker-style process creation cannot escape interception.
    return real_user_process(p,t,pa,ta,po,to,pf,tf,params,info,attrs);
}
template<typename Function, typename App, typename Command, typename Startup, typename Directory>
static BOOL process(Function original,App app,Command command,LPSECURITY_ATTRIBUTES pa,LPSECURITY_ATTRIBUTES ta,BOOL inherit,DWORD flags,LPVOID env,Directory cwd,Startup startup,LPPROCESS_INFORMATION info) {
    if (!initialized || spawning) return original(app,command,pa,ta,inherit,flags,env,cwd,startup,info);
    if(inherit){std::lock_guard<std::recursive_mutex> lock(handles_mutex);for(auto& [handle,item]:handles){(void)item;DWORD attributes=0;if(!GetHandleInformation(handle,&attributes)||(attributes&HANDLE_FLAG_INHERIT)){SetLastError(ERROR_NOT_SUPPORTED);return FALSE;}}}
    if (flags & (CREATE_BREAKAWAY_FROM_JOB | DEBUG_PROCESS | DEBUG_ONLY_THIS_PROCESS | CREATE_PROTECTED_PROCESS)) { SetLastError(ERROR_NOT_SUPPORTED); return FALSE; }
    ++spawning;
    BOOL made = original(app,command,pa,ta,inherit,flags | CREATE_SUSPENDED,env,cwd,startup,info);
    --spawning;
    if (!made) return FALSE;
    Guard guard;
    if (!mb_inject(info->hProcess, mb_config.c_str())) {
        TerminateProcess(info->hProcess, ERROR_DLL_INIT_FAILED); WaitForSingleObject(info->hProcess,1000);
        CloseHandle(info->hThread); CloseHandle(info->hProcess); memset(info,0,sizeof(*info)); SetLastError(ERROR_DLL_INIT_FAILED); return FALSE;
    }
    if (!(flags & CREATE_SUSPENDED) && ResumeThread(info->hThread) == static_cast<DWORD>(-1)) {
        TerminateProcess(info->hProcess, ERROR_DLL_INIT_FAILED); CloseHandle(info->hThread); CloseHandle(info->hProcess); memset(info,0,sizeof(*info)); return FALSE;
    }
    return TRUE;
}
static BOOL WINAPI hook_process_w(LPCWSTR a,LPWSTR c,LPSECURITY_ATTRIBUTES p,LPSECURITY_ATTRIBUTES t,BOOL i,DWORD f,LPVOID e,LPCWSTR d,LPSTARTUPINFOW s,LPPROCESS_INFORMATION o) { return process(real_process_w,a,c,p,t,i,f,e,d,s,o); }
static BOOL WINAPI hook_process_a(LPCSTR a,LPSTR c,LPSECURITY_ATTRIBUTES p,LPSECURITY_ATTRIBUTES t,BOOL i,DWORD f,LPVOID e,LPCSTR d,LPSTARTUPINFOA s,LPPROCESS_INFORMATION o) { return process(real_process_a,a,c,p,t,i,f,e,d,s,o); }

static bool load_config(const wchar_t* path) {
    HANDLE input = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (input == INVALID_HANDLE_VALUE) return false;
    LARGE_INTEGER size{}; DWORD got = 0;
    if (!GetFileSizeEx(input,&size) || size.QuadPart < 16 || size.QuadPart > 100*1024*1024 || size.QuadPart % 2) { CloseHandle(input); return false; }
    std::wstring text(static_cast<size_t>(size.QuadPart)/2,L'\0');
    bool read = ReadFile(input,text.data(),static_cast<DWORD>(size.QuadPart),&got,nullptr) && got == size.QuadPart;
    CloseHandle(input);
    if (!read || !text.starts_with(L"MBVFS1\n") || text.find(L'\0') != std::wstring::npos) return false;
    size_t start = 7, count = 0;
    std::set<std::wstring,Ordinal> validated_sources, checked;
    while (start < text.size()) {
        size_t end = text.find(L'\n',start); if (end == std::wstring::npos) return false;
        auto line = text.substr(start,end-start); start = end+1;
        size_t tab = line.find(L'\t'); if (tab == std::wstring::npos) return false;
        auto kind = line.substr(0,tab), value = line.substr(tab+1);
        if (kind == L"session") { session = value; if (session.size() != 32 || session.find_first_not_of(L"0123456789abcdef") != std::wstring::npos) return false; }
        else if (kind == L"owner") owner = normalize(value);
        else if (kind == L"protect") { auto root = normalize(value); if (root.empty()) return false; protected_roots.insert(root); }
        else if (kind == L"file" || kind == L"mount") {
            tab = value.find(L'\t'); if (tab == std::wstring::npos) return false;
            auto target = normalize(value.substr(0,tab)), source = normalize(value.substr(tab+1));
            if (target.empty() || source.empty() || target.find(L':',2) != std::wstring::npos || source.find(L':',2) != std::wstring::npos || ++count > 500000) return false;
            if ((kind == L"mount" && !safe_path(target)) || (validated_sources.insert(source).second && !safe_snapshot_path(source,checked))) return false;
            auto& collection = kind == L"file" ? files : mounts;
            if (!collection.emplace(target,source).second) return false;
            for (auto descendant = target, dir = parent(target); dir.size() > 3; descendant = dir, dir = parent(dir)) {
                virtual_dirs.insert(dir); children[dir].insert(descendant.substr(dir.size()+1));
            }
        } else return false;
    }
    if (owner.empty() || session.empty() || !safe_path(owner)) return false;
    for (const auto& [target,source] : mounts) { (void)target; if (!within(source,owner) || !safe_path(source,true)) return false; }
    return true;
}
template<typename T> static bool bind(const wchar_t* module, const char* name, void* hook, T& original) {
    void* target = reinterpret_cast<void*>(GetProcAddress(GetModuleHandleW(module),name));
    return target && MH_CreateHook(target,hook,reinterpret_cast<void**>(&original)) == MH_OK;
}
extern "C" __declspec(dllexport) DWORD WINAPI MBVFS_Initialize(void* path) {
    if (initialized) return 0;
    try {
        Guard guard;
        if (!path || !load_config(static_cast<const wchar_t*>(path)) || !initialize_overlay() || MH_Initialize() != MH_OK) return 0;
        wchar_t image_path[32768];
        const DWORD image_length = GetModuleFileNameW(nullptr,image_path,32768);
        game_window_scope = image_length && image_length < 32768 && fallout_image(image_path);
        bool bound = bind(L"ntdll.dll","NtCreateFile",reinterpret_cast<void*>(&hook_create),real_create)
            && bind(L"ntdll.dll","NtOpenFile",reinterpret_cast<void*>(&hook_open),real_open)
            && bind(L"ntdll.dll","NtQueryAttributesFile",reinterpret_cast<void*>(&hook_attributes),real_attributes)
            && bind(L"ntdll.dll","NtQueryFullAttributesFile",reinterpret_cast<void*>(&hook_full_attributes),real_full_attributes)
            && bind(L"ntdll.dll","NtQueryDirectoryFile",reinterpret_cast<void*>(&hook_dir),real_dir)
            && bind(L"ntdll.dll","NtQueryDirectoryFileEx",reinterpret_cast<void*>(&hook_dir_ex),real_dir_ex)
            && bind(L"ntdll.dll","NtSetInformationFile",reinterpret_cast<void*>(&hook_set),real_set)
            && bind(L"ntdll.dll","NtDeleteFile",reinterpret_cast<void*>(&hook_delete),real_delete)
            && bind(L"ntdll.dll","NtClose",reinterpret_cast<void*>(&hook_close),real_close)
            && bind(L"ntdll.dll","NtDuplicateObject",reinterpret_cast<void*>(&hook_duplicate),real_duplicate)
            && bind(L"ntdll.dll","NtCreateUserProcess",reinterpret_cast<void*>(&hook_user_process),real_user_process)
            && bind(L"kernel32.dll","CreateProcessW",reinterpret_cast<void*>(&hook_process_w),real_process_w)
            && bind(L"kernel32.dll","CreateProcessA",reinterpret_cast<void*>(&hook_process_a),real_process_a)
            && bind(L"user32.dll","FindWindowA",reinterpret_cast<void*>(&hook_find_window_a),real_find_window_a)
            && bind(L"user32.dll","FindWindowW",reinterpret_cast<void*>(&hook_find_window_w),real_find_window_w);
        if (!bound) { MH_Uninitialize(); return 0; }
        mb_config = static_cast<const wchar_t*>(path);
        initialized = true;
        if (MH_EnableHook(MH_ALL_HOOKS) != MH_OK) { initialized = false; MH_Uninitialize(); return 0; }
        return 1;
    } catch (...) { return 0; }
}
