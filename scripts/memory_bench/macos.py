"""Non-suspending per-process footprint accounting via Darwin's libproc API."""
from __future__ import annotations

import ctypes
import os
import platform


class RUsageV0(ctypes.Structure):
    _fields_ = [("uuid", ctypes.c_ubyte * 16)] + [
        (field, ctypes.c_uint64) for field in (
            "user_time", "system_time", "pkg_idle_wkups", "interrupt_wkups",
            "pageins", "wired_size", "resident_size", "phys_footprint",
            "proc_start_abstime", "proc_exit_abstime",
        )
    ]


class FootprintReader:
    def __init__(self):
        if platform.system() != "Darwin":
            raise RuntimeError("Footprint accounting requires macOS")
        self.library = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        self.query = self.library.proc_pid_rusage
        self.query.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
        self.query.restype = ctypes.c_int

    def __call__(self, pid: int) -> dict:
        usage = RUsageV0()
        if self.query(pid, 0, ctypes.byref(usage)) != 0:
            code = ctypes.get_errno()
            raise OSError(code, os.strerror(code), str(pid))
        return {"footprint_kib": usage.phys_footprint / 1024,
                "rusage_start_abstime": usage.proc_start_abstime}
