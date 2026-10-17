// Harmless editor/generator corpus. All paths are supplied generated fixtures.
static int tool_checks(const std::wstring& exe,const std::wstring& data,const std::wstring& source,const std::wstring& report) {
    std::vector<std::pair<std::string,bool>> checks;std::vector<std::pair<std::string,DWORD>> refusals;
    auto check=[&](const char* name,bool success){checks.emplace_back(name,success);};
    auto refuse=[&](const char* name,bool success){DWORD error=GetLastError();refusals.emplace_back(name,error);check(name,!success&&error!=0);};
    auto path=[&](const wchar_t* name){return data+L"\\"+name;};
    if(GetFileAttributesW(path(L"crash.marker").c_str())!=INVALID_FILE_ATTRIBUTES) {
        check("crash_partial_tree_reopened",read(path(L"crash-tree\\nested\\chunk.txt"))=="partial generated chunk");
        check("crash_mapping_persisted",read(path(L"mapped-crash.txt"))=="CRASH! payload");
    }
    if(GetFileAttributesW(path(L"tool-pass.marker").c_str())!=INVALID_FILE_ATTRIBUTES) {
        for(auto name:{L"copied.txt",L"replace.bak",L"published\\child.txt",L"published\\nested\\chunk.txt",L"generated\\nested\\chunk.txt"})DeleteFileW(path(name).c_str());
        for(auto name:{L"published\\nested",L"published",L"generated\\nested",L"generated"})RemoveDirectoryW(path(name).c_str());
        write(path(L"mapped.txt"),"mapped! payload",GENERIC_WRITE,CREATE_ALWAYS);
        write(path(L"replace.txt"),"replace original",GENERIC_WRITE,CREATE_ALWAYS);
        bool cleaned=DeleteFileW(path(L"exclusive.txt").c_str())!=FALSE;
        if(!cleaned)refusals.emplace_back("repeat_exclusive_cleanup",GetLastError());
    }
    wchar_t temp_buffer[32768];DWORD temp_count=GetTempPathW(32768,temp_buffer);
    auto temp=temp_count&&temp_count<32768?std::wstring(temp_buffer,temp_count):std::wstring{};
    check("owned_temp_available",!temp.empty());
    auto stage=temp+L"editor.stage";
    check("editor_flush_then_replace",write(stage,"editor saved",GENERIC_WRITE,CREATE_ALWAYS)&&MoveFileExW(stage.c_str(),path(L"editor.txt").c_str(),MOVEFILE_REPLACE_EXISTING|MOVEFILE_WRITE_THROUGH)&&read(path(L"editor.txt"))=="editor saved");
    auto exclusive=path(L"exclusive.stage");write(exclusive,"exclusive saved",GENERIC_WRITE,CREATE_ALWAYS);
    HANDLE exclusive_handle=CreateFileW(exclusive.c_str(),DELETE|GENERIC_READ|GENERIC_WRITE,0,nullptr,OPEN_EXISTING,0,nullptr);
    std::wstring exclusive_name=path(L"exclusive.txt");std::vector<char> rename(sizeof(FILE_RENAME_INFO)+exclusive_name.size()*2,0);DWORD rename_bytes=static_cast<DWORD>(exclusive_name.size()*2);
    memcpy(rename.data()+16,&rename_bytes,4);memcpy(rename.data()+20,exclusive_name.data(),rename_bytes);
    bool exclusive_renamed=exclusive_handle!=INVALID_HANDLE_VALUE&&SetFileInformationByHandle(exclusive_handle,FileRenameInfo,rename.data(),static_cast<DWORD>(rename.size()));
    if(!exclusive_renamed)refusals.emplace_back("exclusive_handle_rename",GetLastError());
    if(exclusive_handle!=INVALID_HANDLE_VALUE)CloseHandle(exclusive_handle);
    check("exclusive_handle_save",exclusive_renamed&&read(exclusive_name)=="exclusive saved");
    write(path(L"replace.stage"),"replace saved",GENERIC_WRITE,CREATE_ALWAYS);
    bool replaced=ReplaceFileW(path(L"replace.txt").c_str(),path(L"replace.stage").c_str(),path(L"replace.bak").c_str(),0,nullptr,nullptr)!=FALSE;
    check("replacefile_backup_save",replaced&&read(path(L"replace.txt"))=="replace saved"&&read(path(L"replace.bak"))=="replace original");
    check("copy_winner_to_generated",CopyFileW(path(L"copy.txt").c_str(),path(L"copied.txt").c_str(),TRUE)&&read(path(L"copied.txt"))=="copy original");
    check("copy_collision_preserved",!CopyFileW(path(L"copy.txt").c_str(),path(L"copied.txt").c_str(),TRUE)&&read(path(L"copied.txt"))=="copy original");
    check("generator_tree",CreateDirectoryW(path(L"generated").c_str(),nullptr)&&CreateDirectoryW(path(L"generated\\nested").c_str(),nullptr)&&write(path(L"generated\\nested\\chunk.txt"),"generated chunk",GENERIC_WRITE,CREATE_NEW));
    bool published=MoveFileExW(path(L"generated").c_str(),path(L"published").c_str(),0)!=FALSE;
    if(!published)refusals.emplace_back("generator_publish_rename",GetLastError());
    check("generator_publish_tree",published&&read(path(L"published\\nested\\chunk.txt"))=="generated chunk");
    check("child_generator",child(exe,path(L"copy.txt"),path(L"published\\child.txt"),L"grandchild")&&read(path(L"published\\child.txt"))=="copy original");
    HANDLE directory=CreateFileW(path(L"inventory").c_str(),FILE_LIST_DIRECTORY,7,nullptr,OPEN_EXISTING,FILE_FLAG_BACKUP_SEMANTICS,nullptr);
    std::set<std::wstring> listed;char directory_buffer[4096];bool query_ok=directory!=INVALID_HANDLE_VALUE;
    if(query_ok) {
        bool first=true;while(GetFileInformationByHandleEx(directory,first?FileIdBothDirectoryRestartInfo:FileIdBothDirectoryInfo,directory_buffer,sizeof(directory_buffer))) {
            first=false;size_t offset=0;
            for(;;){auto entry=reinterpret_cast<FILE_ID_BOTH_DIR_INFO*>(directory_buffer+offset);listed.emplace(entry->FileName,entry->FileNameLength/2);if(!entry->NextEntryOffset)break;offset+=entry->NextEntryOffset;if(offset>=sizeof(directory_buffer)){query_ok=false;break;}}
            if(!query_ok)break;
        }
        query_ok=query_ok&&GetLastError()==ERROR_NO_MORE_FILES;
    }
    check("directory_handle_merged_query",query_ok&&listed.contains(L"virtual.txt")&&listed.contains(L"physical.txt"));
    if(directory!=INVALID_HANDLE_VALUE) {
        check("directory_handle_restart",GetFileInformationByHandleEx(directory,FileIdBothDirectoryRestartInfo,directory_buffer,sizeof(directory_buffer))!=FALSE);
        using Query=NTSTATUS(NTAPI*)(HANDLE,HANDLE,PIO_APC_ROUTINE,PVOID,PIO_STATUS_BLOCK,PVOID,ULONG,FILE_INFORMATION_CLASS,BOOLEAN,PUNICODE_STRING,BOOLEAN);
        auto query=reinterpret_cast<Query>(GetProcAddress(GetModuleHandleW(L"ntdll.dll"),"NtQueryDirectoryFile"));IO_STATUS_BLOCK io{};
        check("unknown_directory_class_refused",query(directory,nullptr,nullptr,nullptr,&io,directory_buffer,sizeof(directory_buffer),static_cast<FILE_INFORMATION_CLASS>(60),FALSE,nullptr,TRUE)==static_cast<NTSTATUS>(0xC00000BB));
        CloseHandle(directory);
    } else {check("directory_handle_restart",false);check("unknown_directory_class_refused",false);}
    HANDLE mapped_file=CreateFileW(path(L"mapped.txt").c_str(),GENERIC_READ|GENERIC_WRITE,7,nullptr,OPEN_EXISTING,0,nullptr);
    HANDLE mapping=mapped_file!=INVALID_HANDLE_VALUE?CreateFileMappingW(mapped_file,nullptr,PAGE_READWRITE,0,0,nullptr):nullptr;
    auto view=mapping?static_cast<char*>(MapViewOfFile(mapping,FILE_MAP_WRITE|FILE_MAP_READ,0,0,0)):nullptr;
    bool mapped=view!=nullptr;
    if(view){memcpy(view,"MAPPED!",7);mapped=FlushViewOfFile(view,0)&&FlushFileBuffers(mapped_file);}
    if(mapped_file!=INVALID_HANDLE_VALUE)CloseHandle(mapped_file);
    check("mapped_write_flush_close_handle",mapped&&read(path(L"mapped.txt"))=="MAPPED! payload");
    if(view){memcpy(view+8,"changed",7);FlushViewOfFile(view,0);}
    check("mapped_view_after_file_close",view&&read(path(L"mapped.txt"))=="MAPPED! changed");
    if(view)UnmapViewOfFile(view);if(mapping)CloseHandle(mapping);
    HANDLE readonly_file=CreateFileW(path(L"readonly.txt").c_str(),GENERIC_READ,7,nullptr,OPEN_EXISTING,0,nullptr);
    HANDLE readonly_mapping=readonly_file!=INVALID_HANDLE_VALUE?CreateFileMappingW(readonly_file,nullptr,PAGE_READONLY,0,0,nullptr):nullptr;
    void* forbidden=readonly_mapping?MapViewOfFile(readonly_mapping,FILE_MAP_WRITE,0,0,0):nullptr;
    check("readonly_mapping_write_refused",readonly_mapping&&!forbidden);if(forbidden)UnmapViewOfFile(forbidden);
    if(readonly_mapping)CloseHandle(readonly_mapping);
    HANDLE private_mapping=readonly_file!=INVALID_HANDLE_VALUE?CreateFileMappingW(readonly_file,nullptr,PAGE_WRITECOPY,0,0,nullptr):nullptr;
    auto private_view=private_mapping?static_cast<char*>(MapViewOfFile(private_mapping,FILE_MAP_COPY,0,0,0)):nullptr;
    if(private_view)memcpy(private_view,"private!",8);
    check("mapped_private_copy_preserves_source",private_view&&read(path(L"readonly.txt"))=="readonly original");
    if(private_view)UnmapViewOfFile(private_view);if(private_mapping)CloseHandle(private_mapping);if(readonly_file!=INVALID_HANDLE_VALUE)CloseHandle(readonly_file);
    HANDLE length_file=CreateFileW(path(L"length.txt").c_str(),GENERIC_READ|GENERIC_WRITE,7,nullptr,OPEN_EXISTING,0,nullptr);
    LARGE_INTEGER end{};end.QuadPart=3;bool truncated=length_file!=INVALID_HANDLE_VALUE&&SetFilePointerEx(length_file,end,nullptr,FILE_BEGIN)&&SetEndOfFile(length_file);if(length_file!=INVALID_HANDLE_VALUE)CloseHandle(length_file);
    check("truncate_existing_winner",truncated&&read(path(L"length.txt"))=="len");
    check("attributes_owned_shadow",SetFileAttributesW(path(L"attributes.txt").c_str(),FILE_ATTRIBUTE_HIDDEN)&&GetFileAttributesW(path(L"attributes.txt").c_str())&FILE_ATTRIBUTE_HIDDEN);
    refuse("hardlink_refused",CreateHardLinkW(path(L"hardlink.txt").c_str(),path(L"copy.txt").c_str(),nullptr)!=FALSE);
    refuse("ads_refused",write(path(L"copy.txt:stream"),"unsafe",GENERIC_WRITE,CREATE_ALWAYS));
    refuse("direct_source_write_refused",write(source+L"\\copy.txt","unsafe",GENERIC_WRITE,CREATE_ALWAYS));
    check("source_readback_intact",read(source+L"\\copy.txt")=="copy original");
    std::string result="{\"checks\":{";bool success=true;for(size_t i=0;i<checks.size();++i){if(i)result+=",";result+="\""+checks[i].first+"\":"+(checks[i].second?"true":"false");success=success&&checks[i].second;}
    result+="},\"refusal_errors\":{";for(size_t i=0;i<refusals.size();++i){if(i)result+=",";result+="\""+refusals[i].first+"\":"+std::to_string(refusals[i].second);}result+="}}";
    write(path(L"tool-pass.marker"),"completed",GENERIC_WRITE,CREATE_ALWAYS);
    write(report,result,GENERIC_WRITE,CREATE_ALWAYS);return success?0:1;
}

static int tool_crash(const std::wstring& data,const std::wstring& report) {
    auto path=[&](const wchar_t* name){return data+L"\\"+name;};
    if(!CreateDirectoryW(path(L"crash-tree").c_str(),nullptr)||!CreateDirectoryW(path(L"crash-tree\\nested").c_str(),nullptr)
       ||!write(path(L"crash-tree\\nested\\chunk.txt"),"partial generated chunk",GENERIC_WRITE,CREATE_NEW))return 61;
    HANDLE file=CreateFileW(path(L"mapped-crash.txt").c_str(),GENERIC_READ|GENERIC_WRITE,7,nullptr,OPEN_EXISTING,0,nullptr);
    HANDLE mapping=file!=INVALID_HANDLE_VALUE?CreateFileMappingW(file,nullptr,PAGE_READWRITE,0,0,nullptr):nullptr;
    auto view=mapping?static_cast<char*>(MapViewOfFile(mapping,FILE_MAP_WRITE,0,0,0)):nullptr;
    if(!view)return 62;memcpy(view,"CRASH!",6);
    if(!FlushViewOfFile(view,0)||!FlushFileBuffers(file))return 63;
    CloseHandle(file); // Deliberately keep the mapping/view alive until death.
    if(!write(path(L"crash.marker"),"flushed before death",GENERIC_WRITE,CREATE_ALWAYS)||!write(report,"owned mapped write and partial tree flushed",GENERIC_WRITE,CREATE_ALWAYS))return 64;
    TerminateProcess(GetCurrentProcess(),196);return 65;
}
