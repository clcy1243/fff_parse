"""find-luts · 扫描 .rdata 段寻找连续 float 数组（疑似 LUT 数据）"""
import struct


def run(program, args):
    # Parameters
    min_len = int(args[0]) if len(args) > 0 else 100
    vmin, vmax = (-10.0, 65535.0)

    memory = program.getMemory()
    rdata = None
    for block in memory.getBlocks():
        if block.getName() in (".rdata", ".data"):
            print(f"段 {block.getName()}: 0x{block.getStart()} – 0x{block.getEnd()} "
                  f"({block.getSize():,} bytes)")
            if rdata is None:
                rdata = block
    if rdata is None:
        print("未找到 .rdata 段")
        return

    # Read all bytes
    start = rdata.getStart()
    size = rdata.getSize()
    data = bytearray(size)
    for i in range(size):
        addr = start.add(i)
        data[i] = memory.getByte(addr) & 0xFF

    print(f"\n扫描 {size:,} 字节找连续 float32 数组 (min_len={min_len})...")

    runs = []
    i = 0
    while i + 4 <= size:
        # Check if this looks like start of a run
        if _is_plausible_run_start(data, i):
            run_len = _scan_run(data, i, vmin, vmax)
            if run_len >= min_len:
                addr = start.add(i)
                first = struct.unpack("<f", bytes(data[i:i+4]))[0]
                last = struct.unpack("<f", bytes(data[i+(run_len-1)*4:i+run_len*4]))[0]
                runs.append((addr, run_len, first, last))
                i += run_len * 4
                continue
        i += 4

    print(f"\n找到 {len(runs)} 个候选 float 数组:")
    print(f"{'Address':<14} {'Length':>8} {'First':>12} {'Last':>12}")
    print("-" * 50)
    for addr, n, first, last in runs[:50]:
        print(f"0x{str(addr):<10} {n:>8} {first:>12.4f} {last:>12.4f}")

    if len(runs) > 50:
        print(f"\n... 共 {len(runs)} 条，只显示前 50 条")


def _is_plausible_run_start(data, off):
    """float 数组应以非零、非 NaN 的 float 开头"""
    try:
        v = struct.unpack("<f", bytes(data[off:off+4]))[0]
    except Exception:
        return False
    if v != v:  # NaN
        return False
    if abs(v) > 1e10:
        return False
    return True


def _scan_run(data, start, vmin, vmax, tolerance=10):
    """从 start 开始扫连续 float，返回长度。允许少量 NaN/out-of-range（<=tolerance 个）再停"""
    count = 0
    out_of_range = 0
    i = start
    while i + 4 <= len(data):
        try:
            v = struct.unpack("<f", bytes(data[i:i+4]))[0]
        except Exception:
            break
        if v != v or abs(v) > 1e10 or not (vmin <= v <= vmax):
            out_of_range += 1
            if out_of_range > tolerance:
                break
        else:
            out_of_range = 0
        count += 1
        i += 4
    return count
