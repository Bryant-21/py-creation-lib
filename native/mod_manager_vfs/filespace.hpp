#pragma once
#include <Windows.h>
#include <winternl.h>
#include <string>

extern HMODULE mb_module;
extern std::wstring mb_config;
bool mb_inject(HANDLE process, const wchar_t* config);
extern "C" __declspec(dllexport) DWORD WINAPI MBVFS_Initialize(void* path);
extern "C" __declspec(dllexport) DWORD WINAPI MBVFS_Inject(HANDLE process, const wchar_t* config);
extern "C" __declspec(dllexport) DWORD WINAPI MBVFS_Version();
