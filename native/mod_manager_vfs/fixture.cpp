// Generated-file acceptance executable; never reads a game installation.
#include <Windows.h>
#include <Psapi.h>
#include <winternl.h>
#include <fstream>
#include <filesystem>
#include <iostream>
#include <set>
#include <string>
#include <vector>

static std::string read(const std::wstring& path) {
    std::ifstream stream{std::filesystem::path(path),std::ios::binary};
    return std::string(std::istreambuf_iterator<char>(stream),{});
}
static bool write(const std::wstring& path, const std::string& text, DWORD access, DWORD disposition) {
    HANDLE file = CreateFileW(path.c_str(),access,FILE_SHARE_READ,nullptr,disposition,FILE_ATTRIBUTE_NORMAL,nullptr);
    if (file == INVALID_HANDLE_VALUE) return false;
    DWORD written = 0;
    bool success = WriteFile(file,text.data(),static_cast<DWORD>(text.size()),&written,nullptr) && written == text.size();
    CloseHandle(file); return success;
}
static std::set<std::wstring> names(const std::wstring& path) {
    WIN32_FIND_DATAW data{};
    HANDLE h = FindFirstFileExW((path+L"\\*").c_str(),FindExInfoBasic,&data,FindExSearchNameMatch,nullptr,FIND_FIRST_EX_LARGE_FETCH);
    std::set<std::wstring> result;
    if (h == INVALID_HANDLE_VALUE) return result;
    do { if (wcscmp(data.cFileName,L".") && wcscmp(data.cFileName,L"..")) result.emplace(data.cFileName); } while (FindNextFileW(h,&data));
    FindClose(h); return result;
}
// FindFirstFileW turns "*.dll" into the DOS wildcard form ("<.dll") before the native query; F4SE scans plugins this way.
static std::set<std::wstring> matching(const std::wstring& path, const wchar_t* pattern) {
    WIN32_FIND_DATAW data{};
    HANDLE h = FindFirstFileW((path+L"\\"+pattern).c_str(),&data);
    std::set<std::wstring> result;
    if (h == INVALID_HANDLE_VALUE) return result;
    do { result.emplace(data.cFileName); } while (FindNextFileW(h,&data));
    FindClose(h); return result;
}
static bool child(const std::wstring& exe,const std::wstring& path,const std::wstring& report,const wchar_t* mode) {
    std::wstring command = L"\""+exe+L"\" "+mode+L" \""+path+L"\" \""+report+L"\"";
    STARTUPINFOW startup{}; startup.cb = sizeof(startup); PROCESS_INFORMATION process{};
    if (!CreateProcessW(exe.c_str(),command.data(),nullptr,nullptr,FALSE,0,nullptr,nullptr,&startup,&process)) return false;
    bool success = WaitForSingleObject(process.hProcess,15000) == WAIT_OBJECT_0;
    DWORD exit = 999; GetExitCodeProcess(process.hProcess,&exit);
    CloseHandle(process.hThread); CloseHandle(process.hProcess);
    return success && !exit;
}
#include "mutation_fixture.inc"
#include "tool_fixture.inc"
static DWORD WINAPI small_stack_files(void* argument) {
    const auto& path = *static_cast<std::wstring*>(argument);
    return read(path+L"\\shared.txt") == "winning" && matching(path,L"*.txt").contains(L"shared.txt") ? 0 : 1;
}
int wmain(int argc,wchar_t** argv) {
    if(argc==5&&!wcscmp(argv[1],L"tools"))return tool_checks(argv[0],argv[2],argv[3],argv[4]);
    if(argc==4&&!wcscmp(argv[1],L"tool-crash"))return tool_crash(argv[2],argv[3]);
    if(argc==4&&!wcscmp(argv[1],L"delete-pause")) {
        using Control=void(WINAPI*)(ULONG,const wchar_t*);
        auto control=reinterpret_cast<Control>(GetProcAddress(GetModuleHandleW(L"modbox_vfs.dll"),"MBVFS_TestControl"));
        if(!control)return 51;control(3,argv[3]);return DeleteFileW(argv[2])?0:52;
    }
    if(argc==4&&!wcscmp(argv[1],L"hidden-tree")) {
        auto source=std::wstring(argv[2]),destination=std::wstring(argv[3]);
        if(!MoveFileExW(source.c_str(),destination.c_str(),0))return 53;
        bool hidden=GetFileAttributesW((destination+L"\\orphan.txt").c_str())==INVALID_FILE_ATTRIBUTES&&names(destination).empty();
        return hidden&&RemoveDirectoryW(destination.c_str())?0:54;
    }
    if(argc==5&&!wcscmp(argv[1],L"lease-wait")) {
        auto events=std::wstring(argv[4]);
        HANDLE held=CreateFileW(argv[2],GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,0,nullptr);
        HANDLE entered=OpenEventW(EVENT_MODIFY_STATE,FALSE,(events+L"_entered").c_str()),release=OpenEventW(SYNCHRONIZE,FALSE,(events+L"_release").c_str());
        if(held==INVALID_HANDLE_VALUE||!entered||!release)return 41;
        SetEvent(entered);bool waited=WaitForSingleObject(release,30000)==WAIT_OBJECT_0;
        CloseHandle(held);CloseHandle(entered);CloseHandle(release);
        return waited&&read(argv[3])=="multiprocess moved"?0:42;
    }
    if(argc==5&&!wcscmp(argv[1],L"contended")) {
        bool blocked=!write(argv[2],"incorrect",GENERIC_WRITE,CREATE_ALWAYS);
        bool moved=MoveFileExW(argv[3],argv[4],0)!=FALSE;
        return blocked&&moved&&read(argv[4])=="multiprocess moved"?0:43;
    }
    if(argc==6&&!wcscmp(argv[1],L"transaction")) {
        using Control=void(WINAPI*)(ULONG,const wchar_t*);
        auto control=reinterpret_cast<Control>(GetProcAddress(GetModuleHandleW(L"modbox_vfs.dll"),"MBVFS_TestControl"));
        if(!control)return 31;
        bool failing=!wcscmp(argv[5],L"fail")||!wcscmp(argv[5],L"directory-fail");
        control(failing?1:2,argv[4]);
        bool moved=MoveFileExW(argv[2],argv[3],MOVEFILE_REPLACE_EXISTING|MOVEFILE_WRITE_THROUGH)!=FALSE;
        if(!wcscmp(argv[5],L"directory-fail"))return !moved&&read(std::wstring(argv[2])+L"\\child.txt")=="transaction child"&&GetFileAttributesW(argv[3])==INVALID_FILE_ATTRIBUTES?0:34;
        if(failing)return !moved&&read(argv[2])=="transaction source"&&read(argv[3])=="transaction destination"?0:32;
        return moved?0:33;
    }
    if(argc==5&&!wcscmp(argv[1],L"mutations"))return mutation_checks(argv[2],argv[3],argv[4]);
    if (argc == 4 && !wcscmp(argv[1],L"benchmark")) {
        LARGE_INTEGER frequency{},start{},end{}; QueryPerformanceFrequency(&frequency);
        auto elapsed = [&]() { QueryPerformanceCounter(&end); return (end.QuadPart-start.QuadPart)*1000.0/frequency.QuadPart; };
        auto data = std::wstring(argv[2]);
        QueryPerformanceCounter(&start);
        bool valid = true;
        for (int i=0;i<10000;++i) valid = valid && read(data+L"\\mapped49999.txt") == "winning";
        double reads = elapsed();
        QueryPerformanceCounter(&start);
        WIN32_FILE_ATTRIBUTE_DATA info{};
        for (int i=0;i<10000;++i) valid = valid && GetFileAttributesExW((data+L"\\mapped49999.txt").c_str(),GetFileExInfoStandard,&info) && info.nFileSizeLow == 7;
        double metadata = elapsed();
        QueryPerformanceCounter(&start);
        auto listed = names(data); double enumeration = elapsed();
        PROCESS_MEMORY_COUNTERS memory{}; memory.cb = sizeof(memory); K32GetProcessMemoryInfo(GetCurrentProcess(),&memory,sizeof(memory));
        std::string result = "{\"valid\":"+std::string(valid?"true":"false")+",\"files\":"+std::to_string(listed.size())+
            ",\"read_10000_ms\":"+std::to_string(reads)+",\"metadata_10000_ms\":"+std::to_string(metadata)+
            ",\"enumeration_ms\":"+std::to_string(enumeration)+",\"working_set\":"+std::to_string(memory.WorkingSetSize)+"}";
        write(argv[3],result,GENERIC_WRITE,CREATE_ALWAYS);
        return valid && listed.size() == 50000 ? 0 : 2;
    }
    if (argc == 4 && (!wcscmp(argv[1],L"child") || !wcscmp(argv[1],L"grandchild"))) {
        auto value = read(argv[2]);
        if (!wcscmp(argv[1],L"child") && !child(argv[0],argv[2],std::wstring(argv[3])+L".grand",L"grandchild")) return 3;
        return write(argv[3],value,GENERIC_WRITE,CREATE_ALWAYS) ? 0 : 2;
    }
    if (argc != 5 || wcscmp(argv[1],L"probe")) return 10;
    std::wstring data = argv[2], source = argv[3], report = argv[4];
    std::vector<std::pair<std::string,bool>> checks;
    auto check = [&](const char* name,bool pass) { checks.emplace_back(name,pass); };
    // GPU drivers perform ordinary file operations on 64 KiB worker stacks.
    HANDLE worker = CreateThread(nullptr,64*1024,small_stack_files,&data,STACK_SIZE_PARAM_IS_A_RESERVATION,nullptr);
    DWORD worker_result = 1;
    bool worker_done = worker && WaitForSingleObject(worker,15000) == WAIT_OBJECT_0 && GetExitCodeThread(worker,&worker_result);
    if(worker)CloseHandle(worker);
    check("small_stack_file_io",worker_done && worker_result == 0);
    check("priority_read",read(data+L"\\shared.txt") == "winning");
    check("case_and_dot",read(data+L"\\virtual\\..\\SHARED.TXT") == "winning");
    check("extended_path",read(L"\\\\?\\"+data+L"\\shared.txt") == "winning");
    check("virtual_nested_read",read(data+L"\\virtual\\mod.txt") == "only");
    auto listed = names(data);
    check("merged_enumeration",listed.contains(L"base.txt") && listed.contains(L"shared.txt") && listed.contains(L"virtual"));
    check("nested_enumeration",names(data+L"\\virtual").contains(L"mod.txt"));
    auto typed = matching(data,L"*.txt");
    check("dos_wildcard_enumeration",typed.contains(L"base.txt") && typed.contains(L"shared.txt") && !typed.contains(L"virtual")
        && matching(data+L"\\virtual",L"*.txt").contains(L"mod.txt") && matching(data,L"shared.t?t").contains(L"shared.txt")
        && matching(data,L"base.*").contains(L"base.txt") && matching(data,L"*.dll").empty());
    WIN32_FILE_ATTRIBUTE_DATA info{};
    check("virtual_metadata",GetFileAttributesExW((data+L"\\virtual\\mod.txt").c_str(),GetFileExInfoStandard,&info) && info.nFileSizeLow == 4);
    check("create_new_collision",!write(data+L"\\shared.txt","incorrect",GENERIC_WRITE,CREATE_NEW) && GetLastError() == ERROR_FILE_EXISTS);
    check("cow_append",write(data+L"\\shared.txt","+owned",FILE_APPEND_DATA,OPEN_EXISTING));
    check("cow_readback",read(data+L"\\shared.txt") == "winning+owned");
    check("direct_source_write_denied",!write(source+L"\\shared.txt","incorrect",GENERIC_WRITE,CREATE_ALWAYS));
    check("source_protected",read(source+L"\\shared.txt") == "winning");
    check("owned_snapshot_write",write(data+L"\\..\\Settings.ini","owned snapshot write",GENERIC_WRITE,CREATE_ALWAYS) && read(data+L"\\..\\Settings.ini") == "owned snapshot write");
    auto game_root = data+L"\\..";
    check("game_root_output",read(game_root+L"\\root.txt") == "root" && write(game_root+L"\\ScreenShot1.png","png",GENERIC_WRITE,CREATE_NEW)
        && read(game_root+L"\\ScreenShot1.png") == "png" && matching(game_root,L"ScreenShot*.png").contains(L"ScreenShot1.png")
        && names(game_root).contains(L"Data") && names(game_root).contains(L"root.txt"));
    check("game_cow",write(data+L"\\base.txt","+owned",FILE_APPEND_DATA,OPEN_EXISTING) && read(data+L"\\base.txt") == "game+owned");
    check("new_directory",CreateDirectoryW((data+L"\\created").c_str(),nullptr) != FALSE);
    check("new_file",write(data+L"\\created\\new.txt","new owned",GENERIC_WRITE,CREATE_NEW));
    check("new_visible",names(data).contains(L"created") && read(data+L"\\created\\new.txt") == "new owned");
    check("delete_recreate",DeleteFileW((data+L"\\shared.txt").c_str()) && write(data+L"\\shared.txt","winning+owned",GENERIC_WRITE,CREATE_NEW) && read(data+L"\\shared.txt") == "winning+owned");
    auto outside = report+L".temporary";
    check("unmanaged_rename_denied",write(outside,"external temp",GENERIC_WRITE,CREATE_ALWAYS) &&
        !MoveFileExW(outside.c_str(),(data+L"\\injected-rename.txt").c_str(),MOVEFILE_REPLACE_EXISTING) && read(outside)=="external temp");
    check("unmanaged_link_denied",!CreateHardLinkW((data+L"\\injected-link.txt").c_str(),outside.c_str(),nullptr));
    check("ordinary_rename_allowed",MoveFileExW(outside.c_str(),(outside+L".renamed").c_str(),MOVEFILE_REPLACE_EXISTING) && read(outside+L".renamed")=="external temp");
    check("child_inheritance",child(argv[0],data+L"\\shared.txt",report+L".child",L"child") && read(report+L".child") == "winning+owned");
    check("grandchild_inheritance",read(report+L".child.grand") == "winning+owned");
    std::string result = "{\"checks\":{";
    bool success = true;
    for (size_t i = 0; i < checks.size(); ++i) {
        if (i) result += ",";
        result += "\""+checks[i].first+"\":"+(checks[i].second ? "true" : "false");
        success = success && checks[i].second;
    }
    result += "}}";
    write(report,result,GENERIC_WRITE,CREATE_ALWAYS);
    return success ? 0 : 1;
}
