// Original ModBox21 code. Only freshly-created manager-owned processes are injected.
#include "filespace.hpp"
#include <TlHelp32.h>
#include <Psapi.h>
#include <cstring>
#include <vector>

HMODULE mb_module = nullptr;
std::wstring mb_config;

static uintptr_t remote_module(HANDLE process, const std::wstring& filename) {
    HANDLE snapshot = INVALID_HANDLE_VALUE;
    for (int attempt = 0; attempt != 10; ++attempt) {
        snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, GetProcessId(process));
        if (snapshot != INVALID_HANDLE_VALUE || GetLastError() != ERROR_BAD_LENGTH) break;
        Sleep(10);
    }
    if (snapshot == INVALID_HANDLE_VALUE) return 0;
    MODULEENTRY32W entry{};
    entry.dwSize = sizeof(entry);
    uintptr_t result = 0;
    if (Module32FirstW(snapshot, &entry)) do {
        if (CompareStringOrdinal(entry.szExePath, -1, filename.c_str(), -1, TRUE) == CSTR_EQUAL) {
            result = reinterpret_cast<uintptr_t>(entry.modBaseAddr);
            break;
        }
    } while (Module32NextW(snapshot, &entry));
    CloseHandle(snapshot);
    return result;
}

static bool call_remote(HANDLE process, LPTHREAD_START_ROUTINE function, void* argument, DWORD* result, DWORD timeout = 10000) {
    HANDLE thread = CreateRemoteThread(process, nullptr, 0, function, argument, 0, nullptr);
    if (!thread) return false;
    DWORD wait = WaitForSingleObject(thread, timeout);
    bool success = wait == WAIT_OBJECT_0 && GetExitCodeThread(thread, result);
    if (!success) {
        // A timed-out initializer must never resume or retain a running injection thread.
        TerminateProcess(process, ERROR_DLL_INIT_FAILED);
        WaitForSingleObject(process, 1000);
    }
    CloseHandle(thread);
    return success;
}

static uintptr_t remote_image(HANDLE process, HMODULE local) {
    wchar_t expected[32768], candidate[32768];
    if (!K32GetMappedFileNameW(GetCurrentProcess(),local,expected,32768)) return 0;
    uintptr_t address = 0;
    MEMORY_BASIC_INFORMATION info{};
    while (VirtualQueryEx(process,reinterpret_cast<void*>(address),&info,sizeof(info)) == sizeof(info)) {
        if (info.State == MEM_COMMIT && info.Type == MEM_IMAGE && info.BaseAddress == info.AllocationBase &&
            K32GetMappedFileNameW(process,info.AllocationBase,candidate,32768) &&
            CompareStringOrdinal(expected,-1,candidate,-1,TRUE) == CSTR_EQUAL)
            return reinterpret_cast<uintptr_t>(info.AllocationBase);
        auto next = reinterpret_cast<uintptr_t>(info.BaseAddress) + info.RegionSize;
        if (next <= address) break;
        address = next;
    }
    return 0;
}

static bool load_remote(HANDLE process, void* filename) {
    // A newly-created suspended process may not have KernelBase/loader lists yet.
    // Locate the already-mapped exact system ntdll image, then call its loader from
    // a tiny original x64 thread thunk after Windows initializes that thread.
    HMODULE ntdll = GetModuleHandleW(L"ntdll.dll");
    auto loader = GetProcAddress(ntdll,"LdrLoadDll");
    uintptr_t remote_ntdll = remote_image(process,ntdll);
    if (!loader || !remote_ntdll) return false;
    auto remote_loader = remote_ntdll + reinterpret_cast<uintptr_t>(loader) - reinterpret_cast<uintptr_t>(ntdll);
    unsigned char code[] = {0x48,0x83,0xec,0x28,0x48,0x31,0xc9,0x48,0x31,0xd2,
        0x49,0xb8,0,0,0,0,0,0,0,0,0x49,0xb9,0,0,0,0,0,0,0,0,
        0x48,0xb8,0,0,0,0,0,0,0,0,0xff,0xd0,0x48,0x83,0xc4,0x28,0xc3};
    void* payload = VirtualAllocEx(process,nullptr,64,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);
    void* thunk = VirtualAllocEx(process,nullptr,sizeof(code),MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);
    if (!payload || !thunk) { if(payload) VirtualFreeEx(process,payload,0,MEM_RELEASE); if(thunk) VirtualFreeEx(process,thunk,0,MEM_RELEASE); return false; }
    wchar_t local_path[32768];
    GetModuleFileNameW(mb_module,local_path,32768);
    UNICODE_STRING text{}; text.Buffer = static_cast<wchar_t*>(filename);
    text.Length = static_cast<USHORT>(wcslen(local_path)*2); text.MaximumLength = text.Length+2;
    uintptr_t unicode = reinterpret_cast<uintptr_t>(payload), module_out = unicode+32;
    memcpy(code+12,&unicode,8); memcpy(code+22,&module_out,8); memcpy(code+32,&remote_loader,8);
    DWORD old = 0, result = 0;
    SIZE_T written = 0;
    bool success = WriteProcessMemory(process,payload,&text,sizeof(text),&written) && written == sizeof(text) &&
        WriteProcessMemory(process,thunk,code,sizeof(code),&written) && written == sizeof(code) &&
        VirtualProtectEx(process,thunk,sizeof(code),PAGE_EXECUTE_READ,&old) && FlushInstructionCache(process,thunk,sizeof(code)) &&
        call_remote(process,reinterpret_cast<LPTHREAD_START_ROUTINE>(thunk),nullptr,&result) && result == 0;
    VirtualFreeEx(process,thunk,0,MEM_RELEASE); VirtualFreeEx(process,payload,0,MEM_RELEASE);
    return success;
}

bool mb_inject(HANDLE process, const wchar_t* config) {
    USHORT target = 0, native = 0;
    if (!IsWow64Process2(process, &target, &native) || target != IMAGE_FILE_MACHINE_UNKNOWN || native != IMAGE_FILE_MACHINE_AMD64)
        return false;
    std::vector<wchar_t> filename(32768);
    if (!GetModuleFileNameW(mb_module, filename.data(), static_cast<DWORD>(filename.size()))) return false;
    size_t dll_size = (wcslen(filename.data()) + 1) * sizeof(wchar_t), config_size = (wcslen(config) + 1) * sizeof(wchar_t);
    if (dll_size > 65536 || config_size > 65536) return false;
    void* memory = VirtualAllocEx(process, nullptr, dll_size + config_size, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if (!memory) return false;
    bool success = false;
    SIZE_T written = 0;
    DWORD result = 0;
    auto remote_config = static_cast<char*>(memory) + dll_size;
    if (WriteProcessMemory(process, memory, filename.data(), dll_size, &written) && written == dll_size &&
        WriteProcessMemory(process, remote_config, config, config_size, &written) && written == config_size &&
        load_remote(process, memory)) {
        uintptr_t remote_dll = remote_module(process, filename.data());
        if (remote_dll) {
            auto initializer = reinterpret_cast<LPTHREAD_START_ROUTINE>(remote_dll + reinterpret_cast<uintptr_t>(&MBVFS_Initialize) - reinterpret_cast<uintptr_t>(mb_module));
            success = call_remote(process, initializer, remote_config, &result,60000) && result == 1;
        }
    }
    VirtualFreeEx(process, memory, 0, MEM_RELEASE);
    if (!success) SetLastError(ERROR_DLL_INIT_FAILED);
    return success;
}

extern "C" __declspec(dllexport) DWORD WINAPI MBVFS_Inject(HANDLE process, const wchar_t* config) {
    return mb_inject(process, config) ? 1 : 0;
}
extern "C" __declspec(dllexport) DWORD WINAPI MBVFS_Version() { return 1; }

BOOL WINAPI DllMain(HINSTANCE module, DWORD reason, LPVOID) {
    if (reason == DLL_PROCESS_ATTACH) {
        mb_module = module;
        DisableThreadLibraryCalls(module);
    }
    return TRUE;  // Initialization/hook installation never happens under the loader lock.
}
