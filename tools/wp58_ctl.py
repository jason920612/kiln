"""Drive the real 26.3 client window by posting window messages (no SendInput: the user's own
mouse and keyboard are never touched) and take PrintWindow screenshots of the game window only.

usage: python tools/wp58_ctl.py PID COMMAND [ARGS...]
  shot FILE                 screenshot of the client area (PrintWindow flag 3)
  key NAME [SECONDS]        press (hold SECONDS, default 0.05) a key: w a s d space shift ctrl e q f t l
                            esc enter tab slash 1..9 up down left right f1 f2 f3 f5
  click left|right [SECONDS]  press a mouse button (hold SECONDS)
  look DX DY                turn the camera by DX,DY mouse counts (needs the window to count as focused)
  type TEXT                 post characters (chat, signs, anvils)
  focus                     post WM_SETFOCUS so the game believes the window is active
  info                      window rect and foreground state
"""

import ctypes
import ctypes.wintypes as wt
import sys
import time

user32 = ctypes.windll.user32
gdi32 = ctypes.windll.gdi32
user32.PostMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]

WM_KEYDOWN, WM_KEYUP, WM_CHAR = 0x100, 0x101, 0x102
WM_MOUSEMOVE, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP = 0x200, 0x201, 0x202, 0x204, 0x205
WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEWHEEL = 0x207, 0x208, 0x20A
WM_SETFOCUS, WM_ACTIVATE = 0x7, 0x6

# name -> (virtual key, scancode, extended)
KEYS = {
    "w": (0x57, 0x11, 0), "a": (0x41, 0x1E, 0), "s": (0x53, 0x1F, 0), "d": (0x44, 0x20, 0),
    "space": (0x20, 0x39, 0), "shift": (0x10, 0x2A, 0), "ctrl": (0x11, 0x1D, 0),
    "e": (0x45, 0x12, 0), "q": (0x51, 0x10, 0), "f": (0x46, 0x21, 0), "t": (0x54, 0x14, 0),
    "l": (0x4C, 0x26, 0), "g": (0x47, 0x22, 0), "esc": (0x1B, 0x01, 0), "enter": (0x0D, 0x1C, 0),
    "tab": (0x09, 0x0F, 0), "slash": (0xBF, 0x35, 0), "backspace": (0x08, 0x0E, 0),
    "up": (0x26, 0x48, 1), "down": (0x28, 0x50, 1), "left": (0x25, 0x4B, 1), "right": (0x27, 0x4D, 1),
    "f1": (0x70, 0x3B, 0), "f2": (0x71, 0x3C, 0), "f3": (0x72, 0x3D, 0), "f5": (0x74, 0x3F, 0),
    "c": (0x43, 0x2E, 0), "x": (0x58, 0x2D, 0), "r": (0x52, 0x13, 0), "b": (0x42, 0x30, 0),
}
for i in range(1, 10):
    KEYS[str(i)] = (0x30 + i, 0x01 + i, 0)


def hwnd_of(pid):
    found = []

    @ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
    def cb(h, _):
        p = wt.DWORD()
        user32.GetWindowThreadProcessId(h, ctypes.byref(p))
        if p.value == pid and user32.IsWindowVisible(h) and user32.GetWindowTextLengthW(h) > 0:
            found.append(h)
        return True

    user32.EnumWindows(cb, 0)
    if not found:
        raise SystemExit(f"no window for pid {pid}")
    return found[0]


def client_size(h):
    r = wt.RECT()
    user32.GetClientRect(h, ctypes.byref(r))
    return r.right - r.left, r.bottom - r.top


def key_lparam(scan, ext, up):
    lp = 1 | (scan << 16) | (ext << 24)
    if up:
        lp |= (1 << 30) | (1 << 31)
    return lp


def key_down(h, name):
    vk, sc, ext = KEYS[name]
    user32.PostMessageW(h, WM_KEYDOWN, vk, key_lparam(sc, ext, False))


def key_up(h, name):
    vk, sc, ext = KEYS[name]
    user32.PostMessageW(h, WM_KEYUP, vk, key_lparam(sc, ext, True))


def press(h, name, seconds=0.05):
    key_down(h, name)
    time.sleep(seconds)
    key_up(h, name)


def pos(h):
    w, hh = client_size(h)
    return (w // 2) | ((hh // 2) << 16)


def click(h, button="left", seconds=0.05):
    down, up, mk = {"left": (WM_LBUTTONDOWN, WM_LBUTTONUP, 1), "right": (WM_RBUTTONDOWN, WM_RBUTTONUP, 2),
                    "middle": (WM_MBUTTONDOWN, WM_MBUTTONUP, 0x10)}[button]
    user32.PostMessageW(h, down, mk, pos(h))
    time.sleep(seconds)
    user32.PostMessageW(h, up, 0, pos(h))


def click_at(h, x, y, button="left", seconds=0.05):
    down, up, mk = {"left": (WM_LBUTTONDOWN, WM_LBUTTONUP, 1), "right": (WM_RBUTTONDOWN, WM_RBUTTONUP, 2)}[button]
    lp = (x & 0xFFFF) | ((y & 0xFFFF) << 16)
    user32.PostMessageW(h, WM_MOUSEMOVE, 0, lp)
    time.sleep(0.03)
    user32.PostMessageW(h, down, mk, lp)
    time.sleep(seconds)
    user32.PostMessageW(h, up, 0, lp)


def look(h, dx, dy):
    w, hh = client_size(h)
    cx, cy = w // 2, hh // 2
    user32.PostMessageW(h, WM_MOUSEMOVE, 0, ((cx + dx) & 0xFFFF) | (((cy + dy) & 0xFFFF) << 16))


def type_text(h, text):
    for ch in text:
        user32.PostMessageW(h, WM_CHAR, ord(ch), 1)
        time.sleep(0.01)


def focus(h):
    user32.PostMessageW(h, WM_ACTIVATE, 1, 0)
    user32.PostMessageW(h, WM_SETFOCUS, 0, 0)


def screenshot(h, out):
    from PIL import Image  # noqa: PLC0415

    w, hh = client_size(h)
    hdc = user32.GetDC(h)
    mem = gdi32.CreateCompatibleDC(hdc)
    bmp = gdi32.CreateCompatibleBitmap(hdc, w, hh)
    gdi32.SelectObject(mem, bmp)
    user32.PrintWindow(h, mem, 3)

    class BMI(ctypes.Structure):
        _fields_ = [("size", wt.DWORD), ("w", wt.LONG), ("h", wt.LONG), ("planes", wt.WORD), ("bpp", wt.WORD),
                    ("comp", wt.DWORD), ("img", wt.DWORD), ("x", wt.LONG), ("y", wt.LONG), ("u", wt.DWORD),
                    ("i", wt.DWORD)]

    bmi = BMI(ctypes.sizeof(BMI), w, -hh, 1, 32, 0, 0, 0, 0, 0, 0)
    buf = ctypes.create_string_buffer(w * hh * 4)
    gdi32.GetDIBits(mem, bmp, 0, hh, buf, ctypes.byref(bmi), 0)
    Image.frombuffer("RGBA", (w, hh), buf, "raw", "BGRA", 0, 1).convert("RGB").save(out)
    gdi32.DeleteObject(bmp)
    gdi32.DeleteDC(mem)
    user32.ReleaseDC(h, hdc)


def main():
    pid = int(sys.argv[1])
    cmd = sys.argv[2]
    a = sys.argv[3:]
    h = hwnd_of(pid)
    if cmd == "shot":
        screenshot(h, a[0])
    elif cmd == "key":
        press(h, a[0], float(a[1]) if len(a) > 1 else 0.05)
    elif cmd == "click":
        click(h, a[0], float(a[1]) if len(a) > 1 else 0.05)
    elif cmd == "look":
        look(h, int(a[0]), int(a[1]))
    elif cmd == "type":
        type_text(h, " ".join(a))
    elif cmd == "focus":
        focus(h)
    elif cmd == "info":
        print("hwnd", h, "size", client_size(h), "foreground", user32.GetForegroundWindow() == h)
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
