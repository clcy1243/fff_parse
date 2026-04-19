"""find-u16-luts · 扫 .rdata 找单调递增的 ushort[N] 数组（胶片 LUT 特征）"""
import struct


def run(program, args):
    expected_len = int(args[0]) if len(args) > 0 else 16384   # 14-bit LUT 默认
    max_val = int(args[1]) if len(args) > 1 else 16383        # 14-bit max
    allow_nonmono = int(args[2]) if len(args) > 2 else 0      # 允许的逆向 step 数

    memory = program.getMemory()
    rdata = None
    for block in memory.getBlocks():
        if block.getName() in (".rdata", ".data"):
            if rdata is None or block.getName() == ".rdata":
                rdata = block

    if rdata is None:
        print("未找到 .rdata")
        return

    start = rdata.getStart()
    size = rdata.getSize()
    print(f".rdata: 0x{start} size={size}, 扫长度={expected_len} ushort 的单调表 (<= {max_val})")

    data = bytearray(size)
    for i in range(size):
        data[i] = memory.getByte(start.add(i)) & 0xFF

    # Each LUT = expected_len * 2 bytes
    bytes_per_lut = expected_len * 2
    candidates = []
    i = 0
    while i + bytes_per_lut < size:
        # Quick check: first ushort near 0, last near max_val
        first = struct.unpack("<H", bytes(data[i:i+2]))[0]
        last = struct.unpack("<H", bytes(data[i + bytes_per_lut - 2:i + bytes_per_lut]))[0]
        mid = struct.unpack("<H", bytes(data[i + bytes_per_lut // 2:i + bytes_per_lut // 2 + 2]))[0]

        if first <= 100 and last >= max_val - 500 and 2000 <= mid <= max_val - 2000:
            # Looks promising; verify monotonicity
            prev = 0
            reversals = 0
            values_max = 0
            for k in range(expected_len):
                v = struct.unpack("<H", bytes(data[i + k*2:i + k*2 + 2]))[0]
                if v > max_val:
                    reversals += expected_len  # break out
                    break
                if v < prev:
                    reversals += 1
                if v > values_max:
                    values_max = v
                prev = v

            if reversals <= allow_nonmono and values_max >= max_val - 200:
                addr = start.add(i)
                # Sample 5 points across
                samples = [struct.unpack("<H", bytes(data[i+k*expected_len//8*2:i+k*expected_len//8*2+2]))[0]
                           for k in [0, 1, 2, 4, 6, 7]]
                candidates.append((addr, samples))
                i += bytes_per_lut
                continue
        i += 2  # next alignment

    print(f"\n找到 {len(candidates)} 个单调 ushort[{expected_len}] 候选:")
    for addr, s in candidates[:20]:
        print(f"  0x{addr}  samples at 1/8, 1/4, 1/2, 3/4, 7/8: {s}")
    if len(candidates) > 20:
        print(f"  ... 共 {len(candidates)}")
