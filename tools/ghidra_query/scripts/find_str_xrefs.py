"""find-str-xrefs · 查指定字符串在代码中被引用的位置（调用者函数）"""


def run(program, args):
    if not args:
        print("用法: ./run.sh find-str-xrefs <string>")
        return
    target = args[0]

    memory = program.getMemory()
    # Search both ASCII and UTF-16-LE
    listing = program.getListing()
    fm = program.getFunctionManager()
    rm = program.getReferenceManager()

    # Find the string in .rdata/.data memory by scanning
    hits = []
    for block in memory.getBlocks():
        if not block.getName().startswith("."):
            continue
        start_addr = block.getStart()
        size = block.getSize()
        # Read whole block
        raw = bytearray(size)
        for i in range(size):
            raw[i] = memory.getByte(start_addr.add(i)) & 0xFF

        # ASCII search
        ascii_bytes = target.encode("ascii")
        idx = 0
        while idx < len(raw):
            p = raw.find(ascii_bytes, idx)
            if p < 0: break
            # Must be null-terminated to be a string
            if p + len(ascii_bytes) < len(raw) and raw[p + len(ascii_bytes)] == 0:
                hits.append((start_addr.add(p), "ASCII"))
            idx = p + 1

        # UTF-16 LE search
        utf16_bytes = target.encode("utf-16-le")
        idx = 0
        while idx < len(raw):
            p = raw.find(utf16_bytes, idx)
            if p < 0: break
            if p + len(utf16_bytes) + 1 < len(raw) and raw[p + len(utf16_bytes)] == 0 and raw[p + len(utf16_bytes) + 1] == 0:
                hits.append((start_addr.add(p), "UTF-16"))
            idx = p + 2

    print(f"找到字符串 '{target}' 共 {len(hits)} 处:")
    for addr, enc in hits[:5]:
        print(f"\n  at 0x{addr} ({enc})")
        # Find references TO this address
        refs = list(rm.getReferencesTo(addr))
        print(f"    引用者 ({len(refs)}):")
        for ref in refs[:20]:
            from_addr = ref.getFromAddress()
            func = fm.getFunctionContaining(from_addr)
            if func:
                parent = func.getParentNamespace()
                pn = parent.getName() if parent and parent.getName() != "Global" else ""
                if pn:
                    pn = f" [{pn}]"
                print(f"      0x{from_addr}  in {func.getName()}{pn} @ 0x{func.getEntryPoint()}")
            else:
                print(f"      0x{from_addr}  (no containing function)")
