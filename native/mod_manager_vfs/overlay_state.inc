// Included by filespace.cpp: one owner-wide lock, persistent whiteouts and
// an atomic rename journal. All I/O here runs under Guard, outside projections.
struct Identity {
    DWORD volume = 0, high = 0, low = 0;
    bool operator==(const Identity&) const = default;
    explicit operator bool() const { return volume || high || low; }
};
static Identity identity(HANDLE handle) {
    BY_HANDLE_FILE_INFORMATION info{};
    if (!GetFileInformationByHandle(handle,&info)) return {};
    return {info.dwVolumeSerialNumber,info.nFileIndexHigh,info.nFileIndexLow};
}
static Identity identity(const std::wstring& path) {
    HANDLE handle = CreateFileW(path.c_str(),FILE_READ_ATTRIBUTES,7,nullptr,OPEN_EXISTING,FILE_FLAG_BACKUP_SEMANTICS|FILE_FLAG_OPEN_REPARSE_POINT,nullptr);
    if (handle == INVALID_HANDLE_VALUE) return {};
    auto result = identity(handle); CloseHandle(handle); return result;
}
constexpr size_t lease_capacity = 1024, lease_path_capacity = 2048;
struct Lease {
    DWORD pid = 0, access = 0, share = 0, directory = 0;
    ULONGLONG birth = 0, token = 0;
    Identity file;
    wchar_t path[lease_path_capacity]{};
};
struct SharedOverlay { ULONGLONG magic = 0, epoch = 0, serial = 0; DWORD journal = 0; Lease leases[lease_capacity]; };
static HANDLE namespace_mutex = nullptr, namespace_mapping = nullptr;
static SharedOverlay* shared_overlay = nullptr;
static ULONGLONG local_epoch = ~0ull, process_birth = 0;
static std::wstring state_root;
static std::set<std::wstring,Ordinal> whiteouts, opaque_dirs;
static std::map<std::wstring,Identity,Ordinal> pending_deletes;
static bool read_bytes(const std::wstring& path, std::vector<char>& bytes, bool absent_ok = false) {
    if (!safe_path(path,true)) return false;
    HANDLE input = CreateFileW(path.c_str(),GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_ATTRIBUTE_NORMAL,nullptr);
    if (input == INVALID_HANDLE_VALUE) return absent_ok && GetLastError() == ERROR_FILE_NOT_FOUND;
    LARGE_INTEGER size{}; DWORD got=0;
    bool valid = GetFileSizeEx(input,&size) && size.QuadPart>=0 && size.QuadPart<=16*1024*1024;
    if (valid) { bytes.resize(static_cast<size_t>(size.QuadPart)); valid=ReadFile(input,bytes.data(),static_cast<DWORD>(bytes.size()),&got,nullptr)&&got==bytes.size(); }
    CloseHandle(input); return valid;
}
static bool atomic_bytes(const std::wstring& path,const std::vector<char>& bytes) {
    if (!safe_path(path,true) || !safe_path(path+L".new",true)) return false;
    HANDLE output=CreateFileW((path+L".new").c_str(),GENERIC_WRITE,0,nullptr,CREATE_ALWAYS,FILE_ATTRIBUTE_NORMAL,nullptr);
    if(output==INVALID_HANDLE_VALUE)return false;
    DWORD wrote=0; bool success=WriteFile(output,bytes.data(),static_cast<DWORD>(bytes.size()),&wrote,nullptr)&&wrote==bytes.size()&&FlushFileBuffers(output);
    CloseHandle(output);
    if(success)success=MoveFileExW((path+L".new").c_str(),path.c_str(),MOVEFILE_REPLACE_EXISTING|MOVEFILE_WRITE_THROUGH)!=FALSE;
    if(!success)DeleteFileW((path+L".new").c_str());
    return success;
}
static bool copy_backup(const std::wstring& source,const std::wstring& destination) {
    if(!safe_path(source,true)||!safe_path(destination,true))return false;
    HANDLE input=CreateFileW(source.c_str(),GENERIC_READ,7,nullptr,OPEN_EXISTING,FILE_FLAG_OPEN_REPARSE_POINT,nullptr);
    if(input==INVALID_HANDLE_VALUE)return false;
    LARGE_INTEGER size{};BY_HANDLE_FILE_INFORMATION info{};
    bool success=GetFileSizeEx(input,&size)&&size.QuadPart<=512ll*1024*1024&&GetFileInformationByHandle(input,&info)&&!(info.dwFileAttributes&FILE_ATTRIBUTE_REPARSE_POINT);
    HANDLE output=success?CreateFileW(destination.c_str(),GENERIC_WRITE,0,nullptr,CREATE_NEW,FILE_ATTRIBUTE_NORMAL,nullptr):INVALID_HANDLE_VALUE;
    success=success&&output!=INVALID_HANDLE_VALUE;
    std::vector<char> buffer(64*1024);DWORD read=0,written=0;ULONGLONG copied=0;
    while(success) {
        if(!ReadFile(input,buffer.data(),static_cast<DWORD>(buffer.size()),&read,nullptr)){success=false;break;}
        if(!read)break;
        if(copied+read>static_cast<ULONGLONG>(size.QuadPart)){success=false;break;}
        if(!WriteFile(output,buffer.data(),read,&written,nullptr)||written!=read){success=false;break;}
        copied+=read;
    }
    if(success)success=copied==static_cast<ULONGLONG>(size.QuadPart)&&SetFileTime(output,&info.ftCreationTime,&info.ftLastAccessTime,&info.ftLastWriteTime)&&FlushFileBuffers(output);
    if(output!=INVALID_HANDLE_VALUE)CloseHandle(output);CloseHandle(input);
    if(success)success=SetFileAttributesW(destination.c_str(),info.dwFileAttributes)!=FALSE;
    if(!success)DeleteFileW(destination.c_str());return success;
}
template<class T> static void append_bytes(std::vector<char>& bytes,const T& value) {
    auto start=reinterpret_cast<const char*>(&value); bytes.insert(bytes.end(),start,start+sizeof(T));
}
static void append_path(std::vector<char>& bytes,const std::wstring& path) {
    append_bytes(bytes,static_cast<DWORD>(path.size()));
    auto start=reinterpret_cast<const char*>(path.data());bytes.insert(bytes.end(),start,start+path.size()*2);
}
struct Reader {
    const std::vector<char>& bytes; size_t offset=0;
    template<class T> T take() { if(sizeof(T)>bytes.size()-offset)throw std::runtime_error("Truncated overlay state"); T value;memcpy(&value,bytes.data()+offset,sizeof(T));offset+=sizeof(T);return value; }
    std::wstring path() {
        auto count=take<DWORD>();if(count>32000||count*2>bytes.size()-offset)throw std::runtime_error("Invalid overlay path");
        std::wstring value(count,L'\0');memcpy(value.data(),bytes.data()+offset,count*2);offset+=count*2;
        if(value.empty()||value.find(L'\0')!=std::wstring::npos||normalize(value)!=value||!within(value,owner)||within(value,state_root)||!safe_path(value))throw std::runtime_error("Unsafe overlay path");
        return value;
    }
};
static std::vector<char> encode_state() {
    std::vector<char> bytes;append_bytes(bytes,0x31534F42u);
    append_bytes(bytes,static_cast<DWORD>(whiteouts.size()));append_bytes(bytes,static_cast<DWORD>(opaque_dirs.size()));append_bytes(bytes,static_cast<DWORD>(pending_deletes.size()));
    for(auto& path:whiteouts)append_path(bytes,path);
    for(auto& path:opaque_dirs)append_path(bytes,path);
    for(auto& [path,id]:pending_deletes){append_path(bytes,path);append_bytes(bytes,id);}
    if(bytes.size()>16*1024*1024||whiteouts.size()+opaque_dirs.size()+pending_deletes.size()>100000)throw std::runtime_error("Overlay state bound exceeded");
    return bytes;
}
static void decode_state(const std::vector<char>& bytes) {
    std::set<std::wstring,Ordinal> whites,opaques;std::map<std::wstring,Identity,Ordinal> pending;
    if(!bytes.empty()) {
        Reader reader{bytes};if(reader.take<DWORD>()!=0x31534F42u)throw std::runtime_error("Invalid overlay state");
        auto nw=reader.take<DWORD>(),no=reader.take<DWORD>(),np=reader.take<DWORD>();
        if(static_cast<ULONGLONG>(nw)+no+np>100000)throw std::runtime_error("Overlay state bound exceeded");
        while(nw--)if(!whites.insert(reader.path()).second)throw std::runtime_error("Duplicate whiteout");
        while(no--)if(!opaques.insert(reader.path()).second)throw std::runtime_error("Duplicate opaque directory");
        while(np--){auto path=reader.path();auto id=reader.take<Identity>();if(!id||!whites.contains(path)||!pending.emplace(path,id).second)throw std::runtime_error("Invalid pending deletion");}
        if(reader.offset!=bytes.size())throw std::runtime_error("Trailing overlay state data");
    }
    whiteouts.swap(whites);opaque_dirs.swap(opaques);pending_deletes.swap(pending);
}
static bool publish_state() {
#ifdef MODBOX_VFS_TESTING
    if(test_publish_failure){test_publish_failure=false;return false;}
#endif
    try {if(!atomic_bytes(state_root+L"\\namespace.bin",encode_state()))return false;}catch(...){return false;}
    local_epoch=++shared_overlay->epoch;return true;
}
static bool live_lease(Lease& lease) {
    if(!lease.pid)return false;
    if(lease.pid==GetCurrentProcessId()&&lease.birth==process_birth)return true;
    HANDLE process=OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION|SYNCHRONIZE,FALSE,lease.pid);
    if(!process){if(GetLastError()==ERROR_INVALID_PARAMETER){lease.pid=0;return false;}return true;}
    FILETIME birth{},exit{},kernel{},user{};
    bool dead=WaitForSingleObject(process,0)==WAIT_OBJECT_0;
    if(GetProcessTimes(process,&birth,&exit,&kernel,&user)){ULONGLONG actual;memcpy(&actual,&birth,8);dead=dead||actual!=lease.birth;}
    CloseHandle(process);if(dead)lease.pid=0;return !dead;
}
static bool generation_open(const std::wstring& path,const Identity& file) {
    for(auto& lease:shared_overlay->leases)if(lease.file==file&&equal(lease.path,path)&&live_lease(lease))return true;
    return false;
}
static bool sharing_allowed(const std::wstring& path,const Identity& file,DWORD access,DWORD share,ULONGLONG own=0) {
    for(auto& lease:shared_overlay->leases)if(lease.token!=own&&lease.file==file&&equal(lease.path,path)&&live_lease(lease)) {
        if((access&~lease.share)||(lease.access&~share))return false;
    }
    return true;
}
static ULONGLONG add_lease(const std::wstring& path,HANDLE handle,DWORD access,DWORD share,bool directory) {
    if(path.size()>=lease_path_capacity)return 0;
    for(auto& lease:shared_overlay->leases)if(!live_lease(lease)) {
        lease={};lease.pid=GetCurrentProcessId();lease.birth=process_birth;lease.token=++shared_overlay->serial;
        lease.file=identity(handle);lease.access=access;lease.share=share;lease.directory=directory;wcscpy_s(lease.path,path.c_str());return lease.token;
    }
    return 0;
}
static void drop_lease(ULONGLONG token) {for(auto& lease:shared_overlay->leases)if(lease.pid==GetCurrentProcessId()&&lease.birth==process_birth&&lease.token==token){lease.pid=0;break;}}
static bool hidden(const std::wstring& path) {
    for(auto current=path;within(current,owner);current=parent(current))if(whiteouts.contains(current))return true;
    return false;
}
static bool lower_hidden(const std::wstring& path) {
    for(auto current=parent(path);within(current,owner);current=parent(current))if(opaque_dirs.contains(current))return true;
    return false;
}
struct RenameJournal {
    std::wstring source,destination,backup; Identity file,destination_file,backup_file;
    std::vector<char> before;
};
static bool write_journal(const RenameJournal& record) {
    std::vector<char> bytes;append_bytes(bytes,0x314A4F42u);append_path(bytes,record.source);append_path(bytes,record.destination);append_path(bytes,record.backup);
    append_bytes(bytes,record.file);append_bytes(bytes,record.destination_file);append_bytes(bytes,record.backup_file);append_bytes(bytes,static_cast<DWORD>(record.before.size()));bytes.insert(bytes.end(),record.before.begin(),record.before.end());
    if(!atomic_bytes(state_root+L"\\rename.bin",bytes))return false;
    shared_overlay->journal=1;return true;
}
static bool clear_journal() {
    if(!DeleteFileW((state_root+L"\\rename.bin").c_str())&&GetLastError()!=ERROR_FILE_NOT_FOUND)return false;
    shared_overlay->journal=0;return true;
}
static bool recover_journal() {
    std::vector<char> bytes;if(!read_bytes(state_root+L"\\rename.bin",bytes,true))return false;
    if(bytes.empty()){shared_overlay->journal=0;return true;}
    Reader reader{bytes};if(reader.take<DWORD>()!=0x314A4F42u)return false;
    // Journal backup lives outside mounts, so Reader's state-root restriction
    // applies to source/destination; the backup is a fixed private filename.
    RenameJournal record;record.source=reader.path();record.destination=reader.path();record.backup=reader.path();
    if(!within(record.backup,owner+L"\\.filespace-cow"))return false;
    record.file=reader.take<Identity>();record.destination_file=reader.take<Identity>();record.backup_file=reader.take<Identity>();auto count=reader.take<DWORD>();
    if(count!=bytes.size()-reader.offset)return false;
    record.before.assign(bytes.begin()+reader.offset,bytes.end());
    auto src=identity(record.source),dst=identity(record.destination);
    if(dst==record.file) {
        if(src)return false;
        if(!MoveFileExW(record.destination.c_str(),record.source.c_str(),MOVEFILE_WRITE_THROUGH))return false;
        dst={};
    }else if(!(src==record.file))return false;
    if(record.destination_file) {
        if(!dst) {
            if(!(identity(record.backup)==record.backup_file)||!MoveFileExW(record.backup.c_str(),record.destination.c_str(),MOVEFILE_WRITE_THROUGH))return false;
        }else if(!(dst==record.destination_file)&&!(dst==record.backup_file))return false;
    }else if(dst)return false;
    if(!atomic_bytes(state_root+L"\\namespace.bin",record.before))return false;
    decode_state(record.before);local_epoch=++shared_overlay->epoch;
    if(!clear_journal())return false;
    DeleteFileW(record.backup.c_str());return true;
}
struct NamespaceLock {
    bool locked=false,valid=false;
    NamespaceLock() {
        if(!namespace_mutex)return;
        DWORD waited=WaitForSingleObject(namespace_mutex,10000);
        locked=waited==WAIT_OBJECT_0||waited==WAIT_ABANDONED;
        if(!locked)return;
        try {
            if(shared_overlay->journal||waited==WAIT_ABANDONED)if(!recover_journal())return;
            if(local_epoch!=shared_overlay->epoch){std::vector<char> bytes;if(!read_bytes(state_root+L"\\namespace.bin",bytes,true))return;decode_state(bytes);local_epoch=shared_overlay->epoch;}
            valid=true;
        }catch(...){valid=false;}
    }
    ~NamespaceLock(){if(locked)ReleaseMutex(namespace_mutex);}
};
static bool initialize_overlay() {
    state_root=owner+L"\\.filespace-state";if(!mkdirs(state_root))return false;
    // Children share their owner's state; each launch must have a distinct owner.
    auto value=folded(owner);unsigned char digest[32];
    BCRYPT_ALG_HANDLE algorithm=nullptr;
    if(BCryptOpenAlgorithmProvider(&algorithm,BCRYPT_SHA256_ALGORITHM,nullptr,0)<0)return false;
    auto status=BCryptHash(algorithm,nullptr,0,reinterpret_cast<PUCHAR>(value.data()),static_cast<ULONG>(value.size()*2),digest,32);BCryptCloseAlgorithmProvider(algorithm,0);if(status<0)return false;
    std::wstring key=L"Local\\ModBoxVFS2_";const wchar_t* hex=L"0123456789abcdef";for(auto byte:digest){key+=hex[byte>>4];key+=hex[byte&15];}
    namespace_mutex=CreateMutexW(nullptr,FALSE,key.c_str());if(!namespace_mutex)return false;
    DWORD waited=WaitForSingleObject(namespace_mutex,10000);if(waited!=WAIT_OBJECT_0&&waited!=WAIT_ABANDONED)return false;
    namespace_mapping=CreateFileMappingW(INVALID_HANDLE_VALUE,nullptr,PAGE_READWRITE,0,sizeof(SharedOverlay),(key+L"_state").c_str());
    if(namespace_mapping)shared_overlay=static_cast<SharedOverlay*>(MapViewOfFile(namespace_mapping,FILE_MAP_ALL_ACCESS,0,0,sizeof(SharedOverlay)));
    bool success=shared_overlay!=nullptr;
    if(success) {
        constexpr ULONGLONG magic=0x32465356424Full;
        if(!shared_overlay->magic){memset(shared_overlay,0,sizeof(SharedOverlay));shared_overlay->magic=magic;shared_overlay->journal=1;}
        if(shared_overlay->magic!=magic)success=false;
        FILETIME birth{},exit{},kernel{},user{};if(!GetProcessTimes(GetCurrentProcess(),&birth,&exit,&kernel,&user))success=false;memcpy(&process_birth,&birth,8);
        if(success)success=recover_journal();
        local_epoch=~0ull;
    }
    ReleaseMutex(namespace_mutex);if(!success)return false;
    NamespaceLock lock;return lock.valid;
}
