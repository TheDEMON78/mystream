// window-audio.exe : capture le son d'une application Windows (et de ses
// processus enfants) grâce à la « capture loopback par processus » de
// WASAPI (Windows 10 version 2004 ou plus récent). C'est la même API que la
// source « Capture audio d'application » d'OBS.
//
// Utilisation :
//   window-audio.exe hwnd:<handle>   son de l'application qui possède la fenêtre
//   window-audio.exe pid:<pid>       son d'un processus
//   window-audio.exe exclude:<pid>   son de tout le système SAUF ce processus
//   window-audio.exe games:<pid>     surveille le jeu au premier plan (voir watchGames)
//
// Sortie : PCM flottant 32 bits, stéréo entrelacé, 48 kHz, sur stdout.
// Le programme s'arrête dès que stdin est fermé (MyStream quitté).

#include <windows.h>
#include <mmdeviceapi.h>
#include <audioclient.h>
#include <fcntl.h>
#include <io.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wchar.h>

// Structures de audioclientactivationparams.h, recopiées sous d'autres noms
// pour compiler aussi bien avec MSVC qu'avec MinGW (qui ne fournit pas l'en-tête).
enum MS_ACTIVATION_TYPE { MS_ACTIVATION_TYPE_DEFAULT = 0, MS_ACTIVATION_TYPE_PROCESS_LOOPBACK = 1 };
enum MS_LOOPBACK_MODE { MS_LOOPBACK_INCLUDE_TREE = 0, MS_LOOPBACK_EXCLUDE_TREE = 1 };
struct MS_LOOPBACK_PARAMS {
  DWORD TargetProcessId;
  MS_LOOPBACK_MODE ProcessLoopbackMode;
};
struct MS_ACTIVATION_PARAMS {
  MS_ACTIVATION_TYPE ActivationType;
  MS_LOOPBACK_PARAMS ProcessLoopbackParams;
};

static const wchar_t *kLoopbackDevice = L"VAD\\Process_Loopback";
static const GUID kIIDUnknown = {0x00000000, 0x0000, 0x0000, {0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46}};
static const GUID kIIDAgileObject = {0x94ea2b94, 0xe9cc, 0x49e0, {0xc0, 0xff, 0xee, 0x64, 0xca, 0x8f, 0x5b, 0x90}};
static const GUID kIIDCompletionHandler = {0x41D949AB, 0x9862, 0x444A, {0x80, 0xF6, 0xC2, 0x61, 0x33, 0x4D, 0xA5, 0xEB}};
static const GUID kIIDAudioClient = {0x1CB9AD4C, 0xDBFA, 0x4c32, {0xB1, 0x78, 0xC2, 0xF5, 0x68, 0xA7, 0x03, 0xB2}};
static const GUID kIIDAudioCaptureClient = {0xC8ADBD64, 0xE71E, 0x48a0, {0xA4, 0xDE, 0x18, 0x5C, 0x39, 0x5C, 0xD3, 0x17}};

typedef HRESULT(WINAPI *ActivateFn)(LPCWSTR, REFIID, PROPVARIANT *, IActivateAudioInterfaceCompletionHandler *,
                                    IActivateAudioInterfaceAsyncOperation **);

// Gestionnaire de fin d'activation : doit être « agile » pour la capture par processus.
class Handler : public IActivateAudioInterfaceCompletionHandler {
 public:
  virtual ~Handler() { CloseHandle(done); }
  HANDLE done = CreateEventW(nullptr, FALSE, FALSE, nullptr);
  HRESULT result = E_FAIL;
  IAudioClient *client = nullptr;

  STDMETHODIMP QueryInterface(REFIID riid, void **ppv) override {
    if (IsEqualGUID(riid, kIIDUnknown) || IsEqualGUID(riid, kIIDCompletionHandler) || IsEqualGUID(riid, kIIDAgileObject)) {
      *ppv = this;
      AddRef();
      return S_OK;
    }
    *ppv = nullptr;
    return E_NOINTERFACE;
  }
  STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&refs_); }
  STDMETHODIMP_(ULONG) Release() override {
    ULONG r = InterlockedDecrement(&refs_);
    if (r == 0) delete this;
    return r;
  }
  STDMETHODIMP ActivateCompleted(IActivateAudioInterfaceAsyncOperation *op) override {
    HRESULT activated = E_FAIL;
    IUnknown *unk = nullptr;
    result = op->GetActivateResult(&activated, &unk);
    if (SUCCEEDED(result)) result = activated;
    if (SUCCEEDED(result)) result = unk->QueryInterface(kIIDAudioClient, reinterpret_cast<void **>(&client));
    if (unk) unk->Release();
    SetEvent(done);
    return S_OK;
  }

 private:
  LONG refs_ = 1;
};

static int fail(const char *msg, HRESULT hr) {
  fprintf(stderr, "%s (0x%08lX)\n", msg, static_cast<unsigned long>(hr));
  return 1;
}

// Quitte quand MyStream ferme stdin (ou se termine).
static DWORD WINAPI watchStdin(LPVOID) {
  char buf[64];
  DWORD n;
  HANDLE in = GetStdHandle(STD_INPUT_HANDLE);
  while (ReadFile(in, buf, sizeof(buf), &n, nullptr) && n > 0) {
  }
  ExitProcess(0);
  return 0;
}

// --- Détection du jeu en plein écran ---------------------------------------

// Fenêtres du système à ne jamais prendre pour un jeu (bureau, barre des tâches…).
static const wchar_t *kIgnoredClasses[] = {L"Progman", L"WorkerW", L"Shell_TrayWnd", L"Shell_SecondaryTrayWnd", L"Windows.UI.Core.CoreWindow", L"XamlExplorerHostIslandWindow"};
static const wchar_t *kIgnoredExes[] = {L"explorer.exe", L"LockApp.exe", L"SearchHost.exe", L"SearchApp.exe", L"ShellExperienceHost.exe", L"StartMenuExperienceHost.exe", L"TextInputHost.exe", L"ScreenClippingHost.exe"};

static void exeName(DWORD pid, wchar_t *out, DWORD size) {
  out[0] = 0;
  HANDLE h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
  if (!h) return;
  wchar_t full[MAX_PATH];
  DWORD n = MAX_PATH;
  if (QueryFullProcessImageNameW(h, 0, full, &n)) {
    const wchar_t *base = wcsrchr(full, L'\\');
    lstrcpynW(out, base ? base + 1 : full, static_cast<int>(size));
  }
  CloseHandle(h);
}

// Comme la « Capture de jeu » d'OBS en mode plein écran : une fenêtre visible
// qui couvre entièrement son écran (plein écran exclusif ou fenêtré sans bordure).
static bool isFullscreenApp(HWND hwnd, DWORD ownPid) {
  if (!hwnd || !IsWindowVisible(hwnd) || IsIconic(hwnd)) return false;
  if (GetWindowLongW(hwnd, GWL_STYLE) & WS_CHILD) return false;
  DWORD pid = 0;
  GetWindowThreadProcessId(hwnd, &pid);
  if (!pid || pid == ownPid || pid == GetCurrentProcessId()) return false;
  wchar_t cls[128];
  GetClassNameW(hwnd, cls, 128);
  for (const wchar_t *c : kIgnoredClasses)
    if (wcscmp(cls, c) == 0) return false;
  wchar_t exe[MAX_PATH];
  exeName(pid, exe, MAX_PATH);
  for (const wchar_t *e : kIgnoredExes)
    if (_wcsicmp(exe, e) == 0) return false;
  RECT r;
  if (!GetWindowRect(hwnd, &r)) return false;
  MONITORINFO mi = {};
  mi.cbSize = sizeof(mi);
  if (!GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mi)) return false;
  const RECT &m = mi.rcMonitor;
  return r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom;
}

static void printUtf8(const wchar_t *text) {
  char buf[1024];
  int n = WideCharToMultiByte(CP_UTF8, 0, text, -1, buf, sizeof(buf), nullptr, nullptr);
  if (n <= 0) return;
  for (char *p = buf; *p; p++)
    if (*p == '\t' || *p == '\n' || *p == '\r') *p = ' ';
  fputs(buf, stdout);
}

// Une ligne par changement : « game <hwnd> <pid>\t<programme>\t<titre> » ou
// « none ». Le jeu reste capturé tant que sa fenêtre existe (alt-tab compris),
// jusqu'à ce qu'une autre application passe en plein écran.
static int watchGames(DWORD ownPid) {
  CreateThread(nullptr, 0, watchStdin, nullptr, 0, nullptr);
  HWND current = nullptr;
  bool announced = false;
  for (;;) {
    HWND fg = GetForegroundWindow();
    HWND next = current;
    if (fg != current && isFullscreenApp(fg, ownPid)) next = fg;
    else if (current && !IsWindow(current)) next = nullptr;
    if (next != current || !announced) {
      current = next;
      announced = true;
      if (current) {
        DWORD pid = 0;
        GetWindowThreadProcessId(current, &pid);
        wchar_t exe[MAX_PATH];
        wchar_t title[256];
        exeName(pid, exe, MAX_PATH);
        GetWindowTextW(current, title, 256);
        printf("game %llu %lu\t", static_cast<unsigned long long>(reinterpret_cast<uintptr_t>(current)), static_cast<unsigned long>(pid));
        printUtf8(exe);
        fputs("\t", stdout);
        printUtf8(title);
        fputs("\n", stdout);
      } else {
        fputs("none\n", stdout);
      }
      fflush(stdout);
    }
    Sleep(1000);
  }
}

int wmain(int argc, wchar_t **argv) {
  if (argc < 2) {
    fprintf(stderr, "Usage : window-audio.exe hwnd:<handle> | pid:<pid> | exclude:<pid> | games:<pid>\n");
    return 2;
  }
  const wchar_t *arg = argv[1];
  if (wcsncmp(arg, L"games:", 6) == 0) return watchGames(wcstoul(arg + 6, nullptr, 10));
  DWORD pid = 0;
  MS_LOOPBACK_MODE mode = MS_LOOPBACK_INCLUDE_TREE;
  if (wcsncmp(arg, L"hwnd:", 5) == 0) {
    HWND hwnd = reinterpret_cast<HWND>(static_cast<uintptr_t>(_wcstoui64(arg + 5, nullptr, 10)));
    GetWindowThreadProcessId(hwnd, &pid);
  } else if (wcsncmp(arg, L"pid:", 4) == 0) {
    pid = wcstoul(arg + 4, nullptr, 10);
  } else if (wcsncmp(arg, L"exclude:", 8) == 0) {
    pid = wcstoul(arg + 8, nullptr, 10);
    mode = MS_LOOPBACK_EXCLUDE_TREE;
  }
  if (!pid) return fail("Fenetre ou processus introuvable", E_INVALIDARG);

  CoInitializeEx(nullptr, COINIT_MULTITHREADED);
  HMODULE mmdev = LoadLibraryW(L"Mmdevapi.dll");
  ActivateFn activate = mmdev ? reinterpret_cast<ActivateFn>(GetProcAddress(mmdev, "ActivateAudioInterfaceAsync")) : nullptr;
  if (!activate) return fail("ActivateAudioInterfaceAsync indisponible", E_NOTIMPL);

  MS_ACTIVATION_PARAMS params = {};
  params.ActivationType = MS_ACTIVATION_TYPE_PROCESS_LOOPBACK;
  params.ProcessLoopbackParams.TargetProcessId = pid;
  params.ProcessLoopbackParams.ProcessLoopbackMode = mode;
  PROPVARIANT pv = {};
  pv.vt = VT_BLOB;
  pv.blob.cbSize = sizeof(params);
  pv.blob.pBlobData = reinterpret_cast<BYTE *>(&params);

  Handler *handler = new Handler();
  IActivateAudioInterfaceAsyncOperation *op = nullptr;
  HRESULT hr = activate(kLoopbackDevice, kIIDAudioClient, &pv, handler, &op);
  if (FAILED(hr)) return fail("Activation impossible (Windows 10 2004 ou plus recent requis)", hr);
  WaitForSingleObject(handler->done, 10000);
  if (FAILED(handler->result) || !handler->client) return fail("Capture du processus refusee", handler->result);
  IAudioClient *client = handler->client;

  WAVEFORMATEX fmt = {};
  fmt.wFormatTag = WAVE_FORMAT_IEEE_FLOAT;
  fmt.nChannels = 2;
  fmt.nSamplesPerSec = 48000;
  fmt.wBitsPerSample = 32;
  fmt.nBlockAlign = fmt.nChannels * fmt.wBitsPerSample / 8;
  fmt.nAvgBytesPerSec = fmt.nSamplesPerSec * fmt.nBlockAlign;
  hr = client->Initialize(AUDCLNT_SHAREMODE_SHARED,
                          AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
                          200000, 0, &fmt, nullptr);
  if (FAILED(hr)) return fail("Initialisation audio impossible", hr);

  HANDLE ready = CreateEventW(nullptr, FALSE, FALSE, nullptr);
  client->SetEventHandle(ready);
  IAudioCaptureClient *capture = nullptr;
  hr = client->GetService(kIIDAudioCaptureClient, reinterpret_cast<void **>(&capture));
  if (FAILED(hr)) return fail("Service de capture indisponible", hr);
  hr = client->Start();
  if (FAILED(hr)) return fail("Demarrage de la capture impossible", hr);

  _setmode(_fileno(stdout), _O_BINARY);
  CreateThread(nullptr, 0, watchStdin, nullptr, 0, nullptr);
  fprintf(stderr, "ok\n");
  fflush(stderr);

  static float silence[48000 * 2];
  for (;;) {
    WaitForSingleObject(ready, 100);
    UINT32 packet = 0;
    while (SUCCEEDED(capture->GetNextPacketSize(&packet)) && packet > 0) {
      BYTE *data = nullptr;
      UINT32 frames = 0;
      DWORD flags = 0;
      if (FAILED(capture->GetBuffer(&data, &frames, &flags, nullptr, nullptr))) break;
      const size_t bytes = static_cast<size_t>(frames) * fmt.nBlockAlign;
      const void *src = (flags & AUDCLNT_BUFFERFLAGS_SILENT) && bytes <= sizeof(silence) ? static_cast<const void *>(silence) : data;
      const size_t written = fwrite(src, 1, bytes, stdout);
      capture->ReleaseBuffer(frames);
      if (written != bytes) return 0;  // MyStream a fermé le tube
    }
    fflush(stdout);
  }
}
