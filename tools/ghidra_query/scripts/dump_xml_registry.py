"""dump-xml-registry · 扫所有 XML key 注册 thunk，输出 key→parser_DAT map

识别 7 条 pattern:
  PUSH <str_addr>       ; 68 xx xx xx xx
  MOV  ECX, <dat_addr>  ; B9 xx xx xx xx
  CALL [0x706b34dc]     ; FF 15 DC 34 6B 70
  PUSH <handler>        ; 68 xx xx xx xx
  CALL _atexit
  POP  ECX
  RET
"""
import struct


def run(program, args):
    memory = program.getMemory()
    listing = program.getListing()

    text_block = None
    for block in memory.getBlocks():
        if block.getName() == ".text":
            text_block = block
            break
    if text_block is None:
        print("no .text block")
        return

    start = text_block.getStart()
    size = text_block.getSize()
    # Read block in chunks
    chunk_size = 0x100000
    needle = bytes([0xFF, 0x15, 0xDC, 0x34, 0x6B, 0x70])  # CALL [0x706b34dc]

    hits = []
    # Scan block in chunks
    for off in range(0, size, chunk_size):
        n = min(chunk_size, size - off)
        buf = bytearray(n)
        for i in range(n):
            buf[i] = memory.getByte(start.add(off + i)) & 0xFF
        pos = 0
        while True:
            p = buf.find(needle, pos)
            if p < 0:
                break
            # Thunk starts 10 bytes before (PUSH + MOV)
            thunk_start = off + p - 10
            if thunk_start >= 0:
                b = bytes(memory.getByte(start.add(thunk_start + i)) & 0xFF for i in range(6))
                if b[0] == 0x68 and b[5] == 0xB9:  # hmm, let me re-check: PUSH imm32 = 68 xx xx xx xx (5 bytes), MOV ECX, imm32 = B9 xx xx xx xx (5 bytes)
                    pass
            # Actually the pattern is: 68 xx xx xx xx (PUSH str, 5 bytes) B9 xx xx xx xx (MOV ECX, 5 bytes) FF 15 DC 34 6B 70 (6 bytes)
            # So thunk starts at off + p - 10 (if first byte is 0x68)
            if thunk_start >= 0:
                first = memory.getByte(start.add(thunk_start)) & 0xFF
                if first == 0x68:
                    str_addr = 0
                    for j in range(4):
                        str_addr |= (memory.getByte(start.add(thunk_start + 1 + j)) & 0xFF) << (j * 8)
                    mov_byte = memory.getByte(start.add(thunk_start + 5)) & 0xFF
                    if mov_byte == 0xB9:
                        dat_addr = 0
                        for j in range(4):
                            dat_addr |= (memory.getByte(start.add(thunk_start + 6 + j)) & 0xFF) << (j * 8)
                        hits.append((thunk_start + int(start.getOffset()), str_addr, dat_addr))
            pos = p + 1

    # For each hit, read the string at str_addr
    def read_cstring(addr_int):
        try:
            from ghidra.program.model.address import AddressFactory  # noqa
            af = program.getAddressFactory()
            a = af.getDefaultAddressSpace().getAddress(addr_int)
            s = bytearray()
            for i in range(200):
                b = memory.getByte(a.add(i)) & 0xFF
                if b == 0:
                    break
                if b < 0x20 or b > 0x7e:
                    return None
                s.append(b)
            return s.decode('ascii', errors='replace')
        except Exception:
            return None

    print(f"Found {len(hits)} XML key registration thunks:\n")
    print(f"{'thunk':<10} {'key_string':<30} {'parser_DAT':<12}")
    print("-" * 60)
    parsed = []
    for thunk, str_a, dat_a in hits:
        s = read_cstring(str_a)
        if s is None:
            continue
        parsed.append((thunk, s, dat_a))
        print(f"0x{thunk:08x} {repr(s):<30} 0x{dat_a:08x}")

    # Group by DAT (parser instance shares = same class)
    print("\n\n按 parser instance 聚类:")
    by_dat = {}
    for thunk, s, dat_a in parsed:
        by_dat.setdefault(dat_a, []).append(s)
    for dat_a, keys in sorted(by_dat.items()):
        if len(keys) > 0:
            print(f"  DAT_0x{dat_a:08x}  ({len(keys)} keys): {keys}")
