// Managed namespace mutations never rename or delete a source/game backing.
static std::wstring virtual_name(const std::wstring& output) {
    std::wstring root,result;
    for(auto& [target,source]:mounts)if(within(output,source)&&source.size()>root.size()){root=source;result=target+output.substr(source.size());}
    if(result.empty())for(auto& [target,source]:files)if(equal(output,source)&&within(source,owner+L"\\runtime")){result=target;break;}
    return result;
}
static bool mount_root(const std::wstring& output) {for(auto& [target,source]:mounts){(void)target;if(equal(output,source))return true;}return false;}
static bool directory_exclusive(const std::wstring& output,ULONGLONG own) {
    for(auto& lease:shared_overlay->leases)if(lease.token!=own&&within(lease.path,output)&&live_lease(lease))return false;
    return true;
}
static bool remove_hidden_output(const std::wstring& output,bool root) {
    std::vector<std::wstring> paths;if(root)paths.push_back(output);
    else {std::set<std::wstring,Ordinal> names;physical_names(output,names);for(auto& name:names)paths.push_back(output+L"\\"+name);}
    // Validate every candidate before cleanup; only already-hidden owned objects
    // may be removed. This also handles a crash before the kernel delete flag.
    for(size_t index=0;index<paths.size();++index) {
        auto& path=paths[index];if(!within(path,owner)||mount_root(path)||!hidden(path)||!safe_path(path,true))return false;
        auto id=identity(path);if(id&&generation_open(path,id))return false;
        DWORD attr=GetFileAttributesW(path.c_str());
        if(attr!=INVALID_FILE_ATTRIBUTES&&(attr&FILE_ATTRIBUTE_DIRECTORY)) {
            std::set<std::wstring,Ordinal> names;physical_names(path,names);
            auto current=path;for(auto& name:names)paths.push_back(current+L"\\"+name);
        }
        if(paths.size()>20000)return false;
    }
    for(auto it=paths.rbegin();it!=paths.rend();++it) {
        DWORD attr=GetFileAttributesW(it->c_str());if(attr==INVALID_FILE_ATTRIBUTES)continue;
        if(!(attr&FILE_ATTRIBUTE_DIRECTORY?RemoveDirectoryW(it->c_str()):DeleteFileW(it->c_str())))return false;
    }
    return true;
}
static bool materialize(const std::wstring& path,const std::wstring& output) {
    struct Copy {std::wstring source,target;bool directory;};
    std::vector<Copy> copies;std::vector<std::pair<std::wstring,std::wstring>> queue{{path,output}};
    ULONGLONG bytes=0;
    for(size_t cursor=0;cursor<queue.size();++cursor) {
        for(auto& entry:enumeration(queue[cursor].first,L"*")) {
            auto source=select(queue[cursor].first+L"\\"+entry.name);
            auto destination=queue[cursor].second+L"\\"+entry.name;
            if(!source.exists||!safe_path(source.physical)||!safe_path(destination,true)||destination.size()>=lease_path_capacity)return false;
            copies.push_back({source.physical,destination,source.directory});
            if(source.directory)queue.emplace_back(queue[cursor].first+L"\\"+entry.name,destination);
            else bytes+=(static_cast<ULONGLONG>(entry.data.nFileSizeHigh)<<32)|entry.data.nFileSizeLow;
            if(copies.size()>20000||bytes>512ull*1024*1024)return false;
        }
    }
    // Planning completes before materialization. Interrupted copies are merely
    // identical owned shadows of still-visible winners, never partial payloads.
    auto staging=owner+L"\\.filespace-cow";if(!mkdirs(staging))return false;
    for(auto& copy:copies) {
        if(copy.directory){if(!mkdirs(copy.target))return false;continue;}
        if(equal(copy.source,copy.target))continue;
        auto temporary=staging+L"\\dir_"+session+L"_"+std::to_wstring(++shared_overlay->serial);
        bool success=CopyFileW(copy.source.c_str(),temporary.c_str(),TRUE)&&safe_path(copy.target,true)&&MoveFileExW(temporary.c_str(),copy.target.c_str(),MOVEFILE_WRITE_THROUGH);
        DeleteFileW(temporary.c_str());if(!success)return false;
    }
    return true;
}
static std::wstring rename_target(PVOID buffer,ULONG length,const std::wstring& source) {
    if(!buffer||length<20)return {};
    ULONG bytes=0;HANDLE root=nullptr;memcpy(&root,static_cast<char*>(buffer)+8,8);memcpy(&bytes,static_cast<char*>(buffer)+16,4);
    if(!bytes||bytes>length-20||bytes>64000||bytes%2)return {};
    std::wstring name(reinterpret_cast<wchar_t*>(static_cast<char*>(buffer)+20),bytes/2);
    if(name.find(L'\0')!=std::wstring::npos)return {};
    if(!root&&name[0]!=L'\\'&&(name.size()<2||name[1]!=L':'))return normalize(parent(source)+L"\\"+name);
    UNICODE_STRING text{};text.Length=static_cast<USHORT>(bytes);text.MaximumLength=text.Length;text.Buffer=name.data();
    OBJECT_ATTRIBUTES attributes{};attributes.Length=sizeof(attributes);attributes.RootDirectory=root;attributes.ObjectName=&text;return object_path(&attributes);
}
static NTSTATUS mutate(HANDLE handle,PIO_STATUS_BLOCK io,PVOID buffer,ULONG length,ULONG cls) {
    std::lock_guard<std::recursive_mutex> lock(handles_mutex);
    auto found=handles.find(handle);if(found==handles.end())return failure(io,unsupported);
    auto opened=found->second;
    if(!(opened->access&(DELETE|GENERIC_ALL))||opened->output.empty()||mount_root(opened->output))return failure(io,denied);
    // File handles follow a rename in another injected process. Recover their
    // current owned name from the kernel rather than stale per-process strings.
    wchar_t physical[32768];DWORD count=GetFinalPathNameByHandleW(handle,physical,32768,FILE_NAME_NORMALIZED|VOLUME_NAME_DOS);
    if(!count||count>=32768)return failure(io,denied);
    auto actual=normalize(physical),logical=virtual_name(actual);
    if(logical.empty()||!within(actual,owner)||!safe_path(actual,true))return failure(io,denied);
    opened->output=actual;opened->path=logical;
    auto generation=identity(handle),current=identity(actual);
    auto source_pending=pending_deletes.find(actual);
    bool current_generation=generation==current||(source_pending!=pending_deletes.end()&&source_pending->second==generation);
    if(cls==13||cls==64) {
        ULONG flags=0;
        if(!buffer||length<(cls==13?1u:4u))return failure(io,unsupported);
        if(cls==13)flags=*static_cast<BOOLEAN*>(buffer)?1u:0u;else memcpy(&flags,buffer,4);
        // Classic deferred unlink and Ex DELETE only. POSIX, ON_CLOSE and
        // readonly/image-section overrides remain explicit unsupported modes.
        if(flags&~1u)return failure(io,unsupported);
        // A displaced destination handle still refers to its detached object.
        // Its disposition must not tombstone a newer generation at this name.
        if(!current_generation)return real_set(handle,io,buffer,length,static_cast<FILE_INFORMATION_CLASS>(cls));
        if(opened->directory&&(flags&1u)) {
            if(!directory_exclusive(actual,opened->lease))return failure(io,static_cast<NTSTATUS>(0xC0000043));
            if(!enumeration(logical,L"*").empty())return failure(io,static_cast<NTSTATUS>(0xC0000101));
            if(!remove_hidden_output(actual,false))return failure(io,denied);
        }
        auto before=encode_state();
        if(flags&1u){whiteouts.insert(actual);pending_deletes[actual]=identity(handle);}
        else{whiteouts.erase(actual);pending_deletes.erase(actual);}
        if(!publish_state()){decode_state(before);return failure(io,denied);}
#ifdef MODBOX_VFS_TESTING
        mb_test_after_delete_state();
#endif
        // State is published before the owned delete flag. If this process dies
        // between them, the committed whiteout still prevents lower resurrection.
        BOOLEAN remove=(flags&1u)!=0;
        NTSTATUS result=real_set(handle,io,&remove,1,static_cast<FILE_INFORMATION_CLASS>(13));
        if(result<0){decode_state(before);if(!publish_state())return failure(io,denied);}
        return result;
    }
    if(cls!=10&&cls!=65)return failure(io,unsupported);
    if(!current_generation)return failure(io,unsupported);
    ULONG flags=0;if(!buffer||length<20)return failure(io,unsupported);
    if(cls==10)flags=*static_cast<BOOLEAN*>(buffer)?1u:0u;else memcpy(&flags,buffer,4);
    if(flags&~1u)return failure(io,unsupported);
    auto target_path=rename_target(buffer,length,logical);if(target_path.empty()||!safe_path(target_path))return failure(io,unsupported);
    auto source=select(logical),target=select(target_path);
    if(!source.exists||!equal(source.physical,actual)||!target.managed||target.output.empty()||mount_root(target.output)||!safe_path(target.output,true))return failure(io,denied);
    if(equal(actual,target.output))return failure(io,unsupported); // case-only rename not yet certified
    if(within(target.output,actual)||within(actual,target.output))return failure(io,unsupported);
    if(auto pending=pending_deletes.find(target.output);pending!=pending_deletes.end()&&generation_open(target.output,pending->second))return failure(io,static_cast<NTSTATUS>(0xC0000056));
    if(pending_deletes.contains(actual))return failure(io,static_cast<NTSTATUS>(0xC0000056));
    if(target.exists&&!(flags&1u))return failure(io,collision);
    if(target.exists&&(opened->directory||target.directory))return failure(io,static_cast<NTSTATUS>(0xC0000035));
    if(opened->directory)for(auto& lease:shared_overlay->leases)
        if(within(lease.path,target.output)&&live_lease(lease))return failure(io,static_cast<NTSTATUS>(0xC0000043));
    auto containing=select(parent(target_path));if(containing.managed&&(!containing.exists||!containing.directory))return failure(io,missing);
    auto file=identity(handle),destination=identity(target.physical);
    if(!sharing_allowed(actual,file,FILE_SHARE_DELETE,7,opened->lease))return failure(io,static_cast<NTSTATUS>(0xC0000043));
    if(target.exists)for(auto& lease:shared_overlay->leases)if(lease.file==destination&&equal(lease.path,target.output)&&live_lease(lease)) {
        if(!(lease.share&FILE_SHARE_DELETE)||(lease.access&FILE_SHARE_WRITE))return failure(io,static_cast<NTSTATUS>(0xC0000043));
    }
    if(opened->directory&&(!directory_exclusive(actual,opened->lease)||!materialize(logical,actual)))return failure(io,static_cast<NTSTATUS>(0xC0000043));
    if(!mkdirs(parent(target.output)))return failure(io,denied);
    if(hidden(target.output)&&GetFileAttributesW(target.output.c_str())!=INVALID_FILE_ATTRIBUTES&&!remove_hidden_output(target.output,true))return failure(io,denied);
    RenameJournal journal{actual,target.output,owner+L"\\.filespace-cow\\rename-backup_"+session+L"_"+std::to_wstring(++shared_overlay->serial),file,identity(target.output),{},encode_state()};
    if(journal.destination_file) {
        if(!mkdirs(parent(journal.backup))||!copy_backup(target.output,journal.backup))return failure(io,denied);
        journal.backup_file=identity(journal.backup);
    }
    if(!write_journal(journal)){DeleteFileW(journal.backup.c_str());return failure(io,denied);}
    auto native=L"\\??\\"+target.output;std::vector<char> request(20+native.size()*2,0);
    // Owned generation replacement uses the kernel's POSIX unlink capability
    // internally. The virtual policy still requires share-delete and rejects
    // destination writers; old permitted handles retain their detached object.
    ULONG physical_cls=cls,physical_flags=flags;
    if(journal.destination_file&&(flags&1u)){physical_cls=65;physical_flags|=2u;}
    ULONG bytes=static_cast<ULONG>(native.size()*2);memcpy(request.data(),&physical_flags,physical_cls==10?1:4);memcpy(request.data()+16,&bytes,4);memcpy(request.data()+20,native.data(),bytes);
    NTSTATUS result=real_set(handle,io,request.data(),static_cast<ULONG>(request.size()),static_cast<FILE_INFORMATION_CLASS>(physical_cls));
    if(result<0){clear_journal();DeleteFileW(journal.backup.c_str());return result;}
#ifdef MODBOX_VFS_TESTING
    mb_test_after_rename();
#endif
    whiteouts.insert(actual);pending_deletes.erase(actual);whiteouts.erase(target.output);pending_deletes.erase(target.output);
    if(opened->directory) {
        // A deleted destination tree's tombstones belong to its old generation.
        // The new tree inherits only the source's hidden descendants, otherwise
        // a repeated generator publish would hide its newly renamed children.
        for(auto it=whiteouts.begin();it!=whiteouts.end();)if(within(*it,target.output))it=whiteouts.erase(it);else++it;
        for(auto it=opaque_dirs.begin();it!=opaque_dirs.end();)if(within(*it,target.output))it=opaque_dirs.erase(it);else++it;
        for(auto it=pending_deletes.begin();it!=pending_deletes.end();)if(within(it->first,target.output))it=pending_deletes.erase(it);else++it;
        std::vector<std::wstring> moved_whiteouts,moved_opaques;
        for(auto& path:whiteouts)if(within(path,actual)&&!equal(path,actual))moved_whiteouts.push_back(target.output+path.substr(actual.size()));
        for(auto& path:opaque_dirs)if(within(path,actual))moved_opaques.push_back(target.output+path.substr(actual.size()));
        whiteouts.insert(moved_whiteouts.begin(),moved_whiteouts.end());opaque_dirs.insert(moved_opaques.begin(),moved_opaques.end());
        opaque_dirs.insert(target.output);
    }
    bool committed=publish_state()&&clear_journal();
    if(!committed){if(!recover_journal())return failure(io,denied);return failure(io,denied);}
    DeleteFileW(journal.backup.c_str());
    for(auto& lease:shared_overlay->leases)if(lease.file==file&&equal(lease.path,actual)&&live_lease(lease))wcscpy_s(lease.path,target.output.c_str());
    for(auto& [key,item]:handles){(void)key;if(equal(item->output,actual)){item->output=target.output;item->path=target_path;item->scanned=false;}}
    return result;
}
