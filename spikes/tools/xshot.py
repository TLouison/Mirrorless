"""Grab the root window of an X display (e.g. Xvfb) to a PNG using only libX11 + stdlib.

Usage: DISPLAY=:99 python3 -I xshot.py out.png
"""
import ctypes
import struct
import sys
import zlib

x11 = ctypes.cdll.LoadLibrary("libX11.so.6")
x11.XOpenDisplay.restype = ctypes.c_void_p
x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
x11.XDefaultRootWindow.restype = ctypes.c_ulong
x11.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
x11.XDefaultScreen.argtypes = [ctypes.c_void_p]
x11.XDisplayWidth.argtypes = [ctypes.c_void_p, ctypes.c_int]
x11.XDisplayHeight.argtypes = [ctypes.c_void_p, ctypes.c_int]
x11.XGetImage.restype = ctypes.c_void_p
x11.XGetImage.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int,
                          ctypes.c_uint, ctypes.c_uint, ctypes.c_ulong, ctypes.c_int]


class XImage(ctypes.Structure):
    _fields_ = [("width", ctypes.c_int), ("height", ctypes.c_int), ("xoffset", ctypes.c_int),
                ("format", ctypes.c_int), ("data", ctypes.POINTER(ctypes.c_ubyte)),
                ("byte_order", ctypes.c_int), ("bitmap_unit", ctypes.c_int),
                ("bitmap_bit_order", ctypes.c_int), ("bitmap_pad", ctypes.c_int),
                ("depth", ctypes.c_int), ("bytes_per_line", ctypes.c_int),
                ("bits_per_pixel", ctypes.c_int)]


def main(out):
    dpy = x11.XOpenDisplay(None)
    if not dpy:
        sys.exit("cannot open display")
    scr = x11.XDefaultScreen(dpy)
    w, h = x11.XDisplayWidth(dpy, scr), x11.XDisplayHeight(dpy, scr)
    ptr = x11.XGetImage(dpy, x11.XDefaultRootWindow(dpy), 0, 0, w, h, 0xFFFFFFFF, 2)  # ZPixmap
    im = ctypes.cast(ptr, ctypes.POINTER(XImage)).contents
    assert im.bits_per_pixel == 32, im.bits_per_pixel
    raw = ctypes.string_at(im.data, im.bytes_per_line * h)
    rows = bytearray()
    for y in range(h):
        line = raw[y * im.bytes_per_line: y * im.bytes_per_line + w * 4]
        rows.append(0)
        px = bytearray(w * 3)
        px[0::3], px[1::3], px[2::3] = line[2::4], line[1::4], line[0::4]  # BGRX -> RGB
        rows += px

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    with open(out, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(bytes(rows), 6)))
        f.write(chunk(b"IEND", b""))


if __name__ == "__main__":
    main(sys.argv[1])
