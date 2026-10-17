#include <Windows.h>
#include <cwchar>

static HWND create_window(const wchar_t* name) {
    WNDCLASSW type{};
    type.hInstance=GetModuleHandleW(nullptr);
    type.lpfnWndProc=DefWindowProcW;
    type.lpszClassName=name;
    if (!RegisterClassW(&type)) return nullptr;
    return CreateWindowExW(0,name,L"Synthetic hidden fixture",0,0,0,10,10,nullptr,nullptr,type.hInstance,nullptr);
}

int wmain(int argc,wchar_t** argv) {
    if (argc==5 && !wcscmp(argv[1],L"server")) {
        const auto window=create_window(argv[2]);
        const auto ready=OpenEventW(EVENT_MODIFY_STATE,FALSE,argv[3]);
        const auto stop=OpenEventW(SYNCHRONIZE,FALSE,argv[4]);
        if (!window || !ready || !stop) return 1;
        SetEvent(ready);
        const auto result=WaitForSingleObject(stop,30000);
        DestroyWindow(window); CloseHandle(ready); CloseHandle(stop);
        return result==WAIT_OBJECT_0 ? 0 : 2;
    }
    if (argc==2 && !wcscmp(argv[1],L"check")) {
        if (FindWindowA("ModBoxGameWindow",nullptr) || FindWindowW(L"ModBoxGameWindow",nullptr)) return 3;
        if (!FindWindowA("ModBoxOtherWindow",nullptr) || !FindWindowW(L"ModBoxOtherWindow",nullptr)) return 4;
        const auto own=create_window(L"ModBoxGameWindow");
        if (!own || FindWindowA("ModBoxGameWindow",nullptr)!=own || FindWindowW(L"ModBoxGameWindow",nullptr)!=own) return 5;
        DestroyWindow(own);
        return 0;
    }
    return 6;
}
