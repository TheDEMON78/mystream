// gpu-capture.exe : capture d'écran ou de fenêtre entièrement sur la carte
// graphique, pour MyStream (Windows 10 version 2004 ou plus récent).
//
// Même technique que la capture d'écran et de fenêtre d'OBS : Windows
// Graphics Capture fournit chaque image sous forme de texture GPU. L'image est
// copiée (sur la carte graphique) dans une texture partagée, dont MyStream
// reçoit le handle et qu'il affiche telle quelle : aucune copie vers le
// processeur, aucune conversion de pixels. La capture intégrée au navigateur
// fait, elle, l'aller-retour GPU -> processeur -> GPU à chaque image, ce qui
// limite la cadence (20 i/s mesurées sur un PC portable pendant un jeu).
//
// Usage : gpu-capture.exe <parent-pid> <monitor|window> <hwnd> <fps> [<vendorId> <deviceId>]
//   monitor : écran qui contient la fenêtre <hwnd> ; window : la fenêtre seule.
//   vendorId/deviceId : carte graphique utilisée par MyStream (sinon la plus puissante).
//
// Sortie (texte, une ligne par message) :
//   slot <id> <handle> <largeur> <hauteur>   nouvelle texture partagée (handle valable dans <parent-pid>)
//   frame <id> <horodatage µs>                une image est prête dans la texture <id>
//   error <message>                           échec, le programme s'arrête
//   closed                                    la fenêtre ou l'écran capturé a disparu
// Entrée : « free <id> » quand MyStream a fini d'utiliser une texture.
// Fermer l'entrée arrête la capture.

#include <windows.h>
#include <d3d11_4.h>
#include <dxgi1_6.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>

#include <atomic>
#include <mutex>
#include <vector>

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Graphics.h>
#include <winrt/Windows.Graphics.Capture.h>
#include <winrt/Windows.Graphics.DirectX.h>
#include <winrt/Windows.Graphics.DirectX.Direct3D11.h>
#include <windows.graphics.capture.interop.h>
#include <windows.graphics.directx.direct3d11.interop.h>

#pragma comment(lib, "d3d11.lib")
#pragma comment(lib, "dxgi.lib")
#pragma comment(lib, "windowsapp.lib")
#pragma comment(lib, "user32.lib")

using namespace winrt;
using namespace winrt::Windows::Graphics::Capture;
using namespace winrt::Windows::Graphics::DirectX;
using namespace winrt::Windows::Graphics::DirectX::Direct3D11;

static const int kSlots = 5;

struct Slot {
  int id = 0;
  com_ptr<ID3D11Texture2D> tex;
  HANDLE remote = nullptr;  // handle dans le processus de MyStream
  std::atomic<bool> busy{false};
};

static std::mutex outLock;
static void say(const char *fmt, ...) {
  std::lock_guard<std::mutex> lock(outLock);
  va_list ap;
  va_start(ap, fmt);
  vprintf(fmt, ap);
  va_end(ap);
  fflush(stdout);
}

static HANDLE parent = nullptr;
static com_ptr<ID3D11Device> d3d;
static com_ptr<ID3D11DeviceContext> ctx;
static com_ptr<ID3D11Query> doneQuery;
static Slot slots[kSlots];
static std::mutex slotLock;
static int nextSlotId = 1;
static int slotW = 0, slotH = 0;

static com_ptr<IDXGIAdapter1> pickAdapter(UINT vendor, UINT device) {
  com_ptr<IDXGIFactory1> factory;
  if (FAILED(CreateDXGIFactory1(__uuidof(IDXGIFactory1), factory.put_void()))) return nullptr;
  com_ptr<IDXGIAdapter1> adapter;
  if (vendor) {
    for (UINT i = 0; factory->EnumAdapters1(i, adapter.put()) != DXGI_ERROR_NOT_FOUND; i++) {
      DXGI_ADAPTER_DESC1 desc;
      if (SUCCEEDED(adapter->GetDesc1(&desc)) && desc.VendorId == vendor && (!device || desc.DeviceId == device)) return adapter;
      adapter = nullptr;
    }
  }
  // Par défaut, la carte graphique la plus puissante (celle que MyStream utilise).
  auto f6 = factory.try_as<IDXGIFactory6>();
  if (f6 && SUCCEEDED(f6->EnumAdapterByGpuPreference(0, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE, __uuidof(IDXGIAdapter1), adapter.put_void()))) return adapter;
  return nullptr;
}

/** (Re)crée les textures partagées à la taille de l'image capturée. */
static bool allocSlots(int w, int h) {
  std::lock_guard<std::mutex> lock(slotLock);
  for (auto &s : slots) {
    // L'ancien handle est fermé dans MyStream : les textures encore affichées restent valides (MyStream en garde sa propre référence).
    if (s.remote) DuplicateHandle(parent, s.remote, nullptr, nullptr, 0, FALSE, DUPLICATE_CLOSE_SOURCE);
    s.remote = nullptr;
    s.tex = nullptr;
    s.busy = false;
  }
  D3D11_TEXTURE2D_DESC desc = {};
  desc.Width = w;
  desc.Height = h;
  desc.MipLevels = 1;
  desc.ArraySize = 1;
  desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
  desc.SampleDesc.Count = 1;
  desc.Usage = D3D11_USAGE_DEFAULT;
  desc.BindFlags = D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET;
  desc.MiscFlags = D3D11_RESOURCE_MISC_SHARED | D3D11_RESOURCE_MISC_SHARED_NTHANDLE;
  for (auto &s : slots) {
    if (FAILED(d3d->CreateTexture2D(&desc, nullptr, s.tex.put()))) return false;
    auto res = s.tex.as<IDXGIResource1>();
    HANDLE local = nullptr;
    if (FAILED(res->CreateSharedHandle(nullptr, DXGI_SHARED_RESOURCE_READ | DXGI_SHARED_RESOURCE_WRITE, nullptr, &local))) return false;
    BOOL ok = DuplicateHandle(GetCurrentProcess(), local, parent, &s.remote, 0, FALSE, DUPLICATE_SAME_ACCESS);
    CloseHandle(local);
    if (!ok) return false;
    s.id = nextSlotId++;
    say("slot %d %llu %d %d\n", s.id, static_cast<unsigned long long>(reinterpret_cast<uintptr_t>(s.remote)), w, h);
  }
  slotW = w;
  slotH = h;
  return true;
}

static Slot *freeSlot() {
  std::lock_guard<std::mutex> lock(slotLock);
  for (auto &s : slots)
    if (s.tex && !s.busy) {
      s.busy = true;
      return &s;
    }
  return nullptr;
}

static DWORD WINAPI readInput(LPVOID) {
  char line[64];
  while (fgets(line, sizeof(line), stdin)) {
    int id = 0;
    if (sscanf(line, "free %d", &id) == 1) {
      std::lock_guard<std::mutex> lock(slotLock);
      for (auto &s : slots)
        if (s.id == id) s.busy = false;
    }
  }
  ExitProcess(0);  // MyStream a fermé l'entrée : fin de la capture
  return 0;
}

int wmain(int argc, wchar_t **argv) {
  if (argc < 5) {
    say("error Usage : gpu-capture.exe <parent-pid> <monitor|window> <hwnd> <fps> [<vendorId> <deviceId>]\n");
    return 2;
  }
  const DWORD parentPid = wcstoul(argv[1], nullptr, 10);
  const bool wholeMonitor = wcscmp(argv[2], L"window") != 0;
  const HWND hwnd = reinterpret_cast<HWND>(static_cast<uintptr_t>(_wcstoui64(argv[3], nullptr, 10)));
  double fps = wcstod(argv[4], nullptr);
  if (fps < 1 || fps > 240) fps = 60;
  const UINT vendor = argc > 5 ? wcstoul(argv[5], nullptr, 10) : 0;
  const UINT device = argc > 6 ? wcstoul(argv[6], nullptr, 10) : 0;

  try {
    init_apartment(apartment_type::multi_threaded);
    if (!GraphicsCaptureSession::IsSupported()) {
      say("error Capture GPU non prise en charge par cette version de Windows\n");
      return 3;
    }
    parent = OpenProcess(PROCESS_DUP_HANDLE, FALSE, parentPid);
    if (!parent) {
      say("error Processus MyStream inaccessible\n");
      return 4;
    }

    com_ptr<IDXGIAdapter1> adapter = pickAdapter(vendor, device);
    const D3D_FEATURE_LEVEL levels[] = {D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1};
    HRESULT hr = D3D11CreateDevice(adapter.get(), adapter ? D3D_DRIVER_TYPE_UNKNOWN : D3D_DRIVER_TYPE_HARDWARE, nullptr, D3D11_CREATE_DEVICE_BGRA_SUPPORT, levels,
                                   ARRAYSIZE(levels), D3D11_SDK_VERSION, d3d.put(), nullptr, ctx.put());
    if (FAILED(hr)) {
      say("error Carte graphique inaccessible (0x%08lx)\n", static_cast<unsigned long>(hr));
      return 5;
    }
    // Le rappel de capture tourne sur un autre fil : contexte protégé.
    if (auto mt = ctx.try_as<ID3D11Multithread>()) mt->SetMultithreadProtected(TRUE);
    D3D11_QUERY_DESC qd = {D3D11_QUERY_EVENT, 0};
    d3d->CreateQuery(&qd, doneQuery.put());

    com_ptr<::IInspectable> inspectable;
    check_hresult(CreateDirect3D11DeviceFromDXGIDevice(d3d.as<IDXGIDevice>().get(), inspectable.put()));
    IDirect3DDevice device3d = inspectable.as<IDirect3DDevice>();

    auto interop = get_activation_factory<GraphicsCaptureItem, IGraphicsCaptureItemInterop>();
    GraphicsCaptureItem item{nullptr};
    if (wholeMonitor) {
      HMONITOR mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY);
      check_hresult(interop->CreateForMonitor(mon, guid_of<IGraphicsCaptureItem>(), put_abi(item)));
    } else {
      check_hresult(interop->CreateForWindow(hwnd, guid_of<IGraphicsCaptureItem>(), put_abi(item)));
    }

    auto size = item.Size();
    if (!allocSlots(size.Width, size.Height)) {
      say("error Textures partagées impossibles à créer\n");
      return 6;
    }

    auto pool = Direct3D11CaptureFramePool::CreateFreeThreaded(device3d, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size);
    auto session = pool.CreateCaptureSession(item);
    try {
      session.IsCursorCaptureEnabled(false);  // pas de curseur sur l'image du jeu
    } catch (...) {
    }
    try {
      session.IsBorderRequired(false);  // Windows 11 : pas de cadre jaune autour de la zone capturée
    } catch (...) {
    }

    LARGE_INTEGER freq, last = {};
    QueryPerformanceFrequency(&freq);
    // Au plus <fps> images par seconde (le jeu peut tourner à 120 i/s ou plus) :
    // une image est retenue si au moins 3/4 d'intervalle s'est écoulé depuis la précédente.
    const LONGLONG minGap = static_cast<LONGLONG>(freq.QuadPart / fps * 0.75);
    winrt::Windows::Graphics::SizeInt32 poolSize = size;

    pool.FrameArrived([&](Direct3D11CaptureFramePool const &sender, winrt::Windows::Foundation::IInspectable const &) {
      try {
      auto frame = sender.TryGetNextFrame();
      if (!frame) return;
      LARGE_INTEGER now;
      QueryPerformanceCounter(&now);
      auto cs = frame.ContentSize();
      if (cs.Width != poolSize.Width || cs.Height != poolSize.Height) {
        // Taille changée (résolution, fenêtre redimensionnée) : nouvelles textures.
        poolSize = cs;
        sender.Recreate(device3d, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, cs);
        if (!allocSlots(cs.Width, cs.Height)) {
          say("error Textures partagées impossibles à recréer\n");
          ExitProcess(6);
        }
        return;
      }
      if (last.QuadPart && now.QuadPart - last.QuadPart < minGap) return;
      Slot *slot = freeSlot();
      if (!slot) return;  // MyStream utilise encore toutes les textures : image sautée
      last = now;
      com_ptr<ID3D11Texture2D> src;
      auto access = frame.Surface().as<::Windows::Graphics::DirectX::Direct3D11::IDirect3DDxgiInterfaceAccess>();
      check_hresult(access->GetInterface(__uuidof(ID3D11Texture2D), src.put_void()));
      D3D11_BOX box = {0, 0, 0, static_cast<UINT>(slotW), static_cast<UINT>(slotH), 1};
      ctx->CopySubresourceRegion(slot->tex.get(), 0, 0, 0, 0, src.get(), 0, &box);
      // On attend la fin de la copie sur la carte graphique avant d'annoncer l'image.
      ctx->End(doneQuery.get());
      ctx->Flush();
      while (ctx->GetData(doneQuery.get(), nullptr, 0, 0) == S_FALSE) YieldProcessor();
      say("frame %d %lld\n", slot->id, static_cast<long long>(frame.SystemRelativeTime().count() / 10));
      } catch (hresult_error const &e) {
        say("error %ls (0x%08lx)\n", e.message().c_str(), static_cast<unsigned long>(e.code()));
        ExitProcess(1);
      }
    });
    item.Closed([](GraphicsCaptureItem const &, winrt::Windows::Foundation::IInspectable const &) {
      say("closed\n");
      ExitProcess(0);
    });

    session.StartCapture();
    CreateThread(nullptr, 0, readInput, nullptr, 0, nullptr);
    // Le fil principal attend : la capture vit dans les rappels.
    for (;;) Sleep(INFINITE);
  } catch (hresult_error const &e) {
    say("error %ls (0x%08lx)\n", e.message().c_str(), static_cast<unsigned long>(e.code()));
    return 1;
  } catch (...) {
    say("error Erreur inconnue\n");
    return 1;
  }
}
