"""list-rtti-classes · 扫 .data 段的 '.?AV...' MSVC RTTI type descriptor 符号

MSVC 编译器把每个 C++ 类的 type descriptor 存为字符串 `.?AVClassName@@`。
直接从内存扫这些字符串能把所有类枚举出来，不依赖 Ghidra 的 namespace 解析。
"""


def run(program, args):
    prefix = args[0] if args else ""  # 可选过滤前缀（如 "C" 只看 C-class）
    memory = program.getMemory()

    hits = []
    for block in memory.getBlocks():
        name = block.getName()
        if name not in (".data", ".rdata"):
            continue
        start = block.getStart()
        size = block.getSize()
        raw = bytearray(size)
        for i in range(size):
            raw[i] = memory.getByte(start.add(i)) & 0xFF

        needle = b".?AV"
        idx = 0
        while idx < len(raw):
            p = raw.find(needle, idx)
            if p < 0:
                break
            # Read until @@\0
            end = raw.find(b"@@\x00", p, min(p + 200, len(raw)))
            if end > 0:
                try:
                    s = bytes(raw[p:end + 2]).decode('ascii')
                    class_name = s[4:-2]  # strip .?AV and @@
                    if prefix == "" or class_name.startswith(prefix):
                        hits.append((start.add(p), class_name))
                except Exception:
                    pass
                idx = end + 3
            else:
                idx = p + 4

    # Dedupe (some classes appear multiple times)
    seen = set()
    unique = []
    for addr, n in hits:
        if n not in seen:
            seen.add(n)
            unique.append((addr, n))

    print(f"Found {len(unique)} unique classes (of {len(hits)} total hits):\n")
    for addr, n in sorted(unique, key=lambda x: x[1]):
        print(f"  0x{addr}  {n}")
