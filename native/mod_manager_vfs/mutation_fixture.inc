// Real Win32 API journeys over generated files, called only by the QA binary.
static std::string handle_read(HANDLE handle) {
    LARGE_INTEGER zero{};SetFilePointerEx(handle,zero,nullptr,FILE_BEGIN);char buffer[256];DWORD got=0;
    return ReadFile(handle,buffer,sizeof(buffer),&got,nullptr)?std::string(buffer,got):std::string{};
}
static bool disposition(HANDLE handle,bool remove) { FILE_DISPOSITION_INFO info{static_cast<BOOLEAN>(remove)};bool result=SetFileInformationByHandle(handle,FileDispositionInfo,&info,sizeof(info))!=FALSE;if(!result)std::cerr<<"disposition "<<remove<<" error "<<GetLastError()<<"\n";return result; }
static int mutation_checks(const std::wstring& data,const std::wstring& temp,const std::wstring& report) {
    std::vector<std::pair<std::string,bool>> checks;
    auto check=[&](const char* name,bool pass){checks.emplace_back(name,pass);};
    auto path=[&](const wchar_t* name){return data+L"\\"+name;};
    check("delete_winner",DeleteFileW(path(L"delete.txt").c_str())&&GetFileAttributesW(path(L"delete.txt").c_str())==INVALID_FILE_ATTRIBUTES&&!names(data).contains(L"delete.txt"));
    check("delete_recreate",write(path(L"delete.txt"),"recreated",GENERIC_WRITE,CREATE_NEW)&&read(path(L"delete.txt"))=="recreated");
    check("delete_again",DeleteFileW(path(L"delete.txt").c_str()));
    HANDLE pinned=CreateFileW(path(L"atomic.txt").c_str(),GENERIC_READ|DELETE,7,nullptr,OPEN_EXISTING,0,nullptr);
    check("temp_to_data_replace",write(temp+L"\\atomic.new","atomic replacement",GENERIC_WRITE,CREATE_NEW)&&MoveFileExW((temp+L"\\atomic.new").c_str(),path(L"atomic.txt").c_str(),MOVEFILE_REPLACE_EXISTING|MOVEFILE_WRITE_THROUGH)&&read(path(L"atomic.txt"))=="atomic replacement");
    check("old_handle_pinned",pinned!=INVALID_HANDLE_VALUE&&handle_read(pinned)=="original atomic");
    if(pinned!=INVALID_HANDLE_VALUE){disposition(pinned,true);CloseHandle(pinned);}
    check("detached_delete_preserves_replacement",read(path(L"atomic.txt"))=="atomic replacement");
    check("atomic_source_gone",GetFileAttributesW((temp+L"\\atomic.new").c_str())==INVALID_FILE_ATTRIBUTES);
    check("rename_winner",MoveFileExW(path(L"rename.txt").c_str(),path(L"renamed.txt").c_str(),0)&&read(path(L"renamed.txt"))=="original rename"&&GetFileAttributesW(path(L"rename.txt").c_str())==INVALID_FILE_ATTRIBUTES);
    check("rename_collision_unchanged",!MoveFileExW(path(L"renamed.txt").c_str(),path(L"atomic.txt").c_str(),0)&&read(path(L"renamed.txt"))=="original rename"&&read(path(L"atomic.txt"))=="atomic replacement");
    check("directory_rename_merged",MoveFileExW(path(L"tree").c_str(),path(L"moved").c_str(),0)&&read(path(L"moved\\winner.txt"))=="tree winner"&&read(path(L"moved\\base.txt"))=="tree base"&&read(path(L"moved\\nested\\child.txt"))=="nested child"&&GetFileAttributesW(path(L"tree").c_str())==INVALID_FILE_ATTRIBUTES);
    check("directory_delete_nonempty_denied",!RemoveDirectoryW(path(L"moved").c_str())&&read(path(L"moved\\winner.txt"))=="tree winner");
    check("directory_recreate_opaque",CreateDirectoryW(path(L"tree").c_str(),nullptr)&&names(path(L"tree")).empty()&&GetFileAttributesW(path(L"tree\\winner.txt").c_str())==INVALID_FILE_ATTRIBUTES);
    check("empty_directory_delete",RemoveDirectoryW(path(L"empty").c_str())&&GetFileAttributesW(path(L"empty").c_str())==INVALID_FILE_ATTRIBUTES);
    HANDLE held=CreateFileW(path(L"sharing.txt").c_str(),GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,0,nullptr);
    check("lower_sharing_cow_denied",held!=INVALID_HANDLE_VALUE&&!write(path(L"sharing.txt"),"incorrect",GENERIC_WRITE,CREATE_ALWAYS)&&handle_read(held)=="sharing original");if(held!=INVALID_HANDLE_VALUE)CloseHandle(held);
    HANDLE lower_pin=CreateFileW(path(L"sharing.txt").c_str(),GENERIC_READ,7,nullptr,OPEN_EXISTING,0,nullptr);
    check("sharing_release_allows_write",write(path(L"sharing.txt"),"sharing owned",GENERIC_WRITE,CREATE_ALWAYS)&&read(path(L"sharing.txt"))=="sharing owned");
    check("lower_handle_pinned_after_cow",lower_pin!=INVALID_HANDLE_VALUE&&handle_read(lower_pin)=="sharing original");if(lower_pin!=INVALID_HANDLE_VALUE)CloseHandle(lower_pin);
    HANDLE removing=CreateFileW(path(L"cancel.txt").c_str(),DELETE|GENERIC_READ,7,nullptr,OPEN_EXISTING,0,nullptr);
    bool pending=removing!=INVALID_HANDLE_VALUE&&disposition(removing,true);
    HANDLE reopen=CreateFileW(path(L"cancel.txt").c_str(),GENERIC_READ,7,nullptr,OPEN_EXISTING,0,nullptr);
    check("pending_delete_open_denied",pending&&reopen==INVALID_HANDLE_VALUE&&(GetLastError()==ERROR_ACCESS_DENIED||GetLastError()==ERROR_DELETE_PENDING));if(reopen!=INVALID_HANDLE_VALUE)CloseHandle(reopen);
    bool cancelled=pending&&disposition(removing,false);if(removing!=INVALID_HANDLE_VALUE)CloseHandle(removing);
    check("cancel_delete_restores",cancelled&&read(path(L"cancel.txt"))=="cancel original");
    HANDLE onclose=CreateFileW(path(L"onclose.txt").c_str(),DELETE|GENERIC_READ,7,nullptr,OPEN_EXISTING,FILE_FLAG_DELETE_ON_CLOSE,nullptr);
    check("delete_on_close_handle",onclose!=INVALID_HANDLE_VALUE&&handle_read(onclose)=="onclose original");if(onclose!=INVALID_HANDLE_VALUE)CloseHandle(onclose);
    check("delete_on_close_whiteout",GetFileAttributesW(path(L"onclose.txt").c_str())==INVALID_FILE_ATTRIBUTES);
    HANDLE directory=CreateFileW(path(L"moved").c_str(),FILE_LIST_DIRECTORY,7,nullptr,OPEN_EXISTING,FILE_FLAG_BACKUP_SEMANTICS,nullptr);
    check("open_descendant_rename_denied",directory!=INVALID_HANDLE_VALUE&&!MoveFileExW(path(L"moved").c_str(),path(L"blocked").c_str(),0)&&read(path(L"moved\\winner.txt"))=="tree winner");if(directory!=INVALID_HANDLE_VALUE)CloseHandle(directory);
    check("directory_delete_children",DeleteFileW(path(L"moved\\winner.txt").c_str())&&DeleteFileW(path(L"moved\\base.txt").c_str())&&DeleteFileW(path(L"moved\\nested\\child.txt").c_str())&&RemoveDirectoryW(path(L"moved\\nested").c_str())&&RemoveDirectoryW(path(L"moved").c_str()));
    write(path(L"relative-source.txt"),"relative original",GENERIC_WRITE,CREATE_NEW);
    HANDLE relative=CreateFileW(path(L"relative-source.txt").c_str(),DELETE|SYNCHRONIZE,7,nullptr,OPEN_EXISTING,0,nullptr);
    HANDLE root=CreateFileW(data.c_str(),FILE_LIST_DIRECTORY,7,nullptr,OPEN_EXISTING,FILE_FLAG_BACKUP_SEMANTICS,nullptr);
    using Set=NTSTATUS(NTAPI*)(HANDLE,PIO_STATUS_BLOCK,PVOID,ULONG,FILE_INFORMATION_CLASS);
    auto native_set=reinterpret_cast<Set>(GetProcAddress(GetModuleHandleW(L"ntdll.dll"),"NtSetInformationFile"));
    std::wstring basename=L"relative-target.txt";std::vector<char> request(20+basename.size()*2,0);
    DWORD bytes=static_cast<DWORD>(basename.size()*2),replace=1;memcpy(request.data(),&replace,4);memcpy(request.data()+8,&root,8);memcpy(request.data()+16,&bytes,4);memcpy(request.data()+20,basename.data(),bytes);
    IO_STATUS_BLOCK status{};bool renamed=relative!=INVALID_HANDLE_VALUE&&root!=INVALID_HANDLE_VALUE&&native_set(relative,&status,request.data(),static_cast<ULONG>(request.size()),static_cast<FILE_INFORMATION_CLASS>(65))>=0;
    if(relative!=INVALID_HANDLE_VALUE)CloseHandle(relative);if(root!=INVALID_HANDLE_VALUE)CloseHandle(root);
    check("rename_ex_root_relative",renamed&&read(path(L"relative-target.txt"))=="relative original");
    write(path(L"duplicate-delete.txt"),"duplicate original",GENERIC_WRITE,CREATE_NEW);
    HANDLE first=CreateFileW(path(L"duplicate-delete.txt").c_str(),DELETE|GENERIC_READ,7,nullptr,OPEN_EXISTING,0,nullptr),duplicate=nullptr;
    bool duplicated=first!=INVALID_HANDLE_VALUE&&DuplicateHandle(GetCurrentProcess(),first,GetCurrentProcess(),&duplicate,0,FALSE,DUPLICATE_SAME_ACCESS);
    bool deleted=duplicated&&disposition(first,true);if(first!=INVALID_HANDLE_VALUE)CloseHandle(first);
    check("duplicate_handle_pending_lifetime",deleted&&handle_read(duplicate)=="duplicate original"&&!write(path(L"duplicate-delete.txt"),"incorrect",GENERIC_WRITE,CREATE_NEW));
    if(duplicate)CloseHandle(duplicate);
    check("duplicate_final_close_recreate",write(path(L"duplicate-delete.txt"),"duplicate recreated",GENERIC_WRITE,CREATE_NEW)&&read(path(L"duplicate-delete.txt"))=="duplicate recreated");
    check("rename_into_deleted_name",MoveFileExW(path(L"relative-target.txt").c_str(),path(L"delete.txt").c_str(),0)&&read(path(L"delete.txt"))=="relative original");
    check("deleted_name_removed_again",DeleteFileW(path(L"delete.txt").c_str()));
    std::vector<HANDLE> capacity;
    for(size_t index=0;index<1024;++index){HANDLE handle=CreateFileW(path(L"atomic.txt").c_str(),GENERIC_READ,7,nullptr,OPEN_EXISTING,0,nullptr);if(handle==INVALID_HANDLE_VALUE)break;capacity.push_back(handle);}
    check("lease_bound_fails_before_create",capacity.size()==1024&&!write(path(L"capacity-ghost.txt"),"incorrect",GENERIC_WRITE,CREATE_NEW)&&GetFileAttributesW(path(L"capacity-ghost.txt").c_str())==INVALID_FILE_ATTRIBUTES);
    for(auto handle:capacity)CloseHandle(handle);
    check("lease_bound_release_recovers",write(path(L"capacity-good.txt"),"capacity owned",GENERIC_WRITE,CREATE_NEW));
    std::string result="{\"checks\":{";bool success=true;
    for(size_t i=0;i<checks.size();++i){if(i)result+=",";result+="\""+checks[i].first+"\":"+(checks[i].second?"true":"false");success=success&&checks[i].second;}
    result+="}}";write(report,result,GENERIC_WRITE,CREATE_ALWAYS);return success?0:1;
}
