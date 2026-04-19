"""find_offset_refs - find instructions that reference a specific structure offset.

Usage: find-offset-refs <hex_offset> [segment_prefix]
Example: find-offset-refs 0x4fc 0x702
Scans instructions for memory operands with the given offset as displacement.
"""
def run(program, args):
    if not args:
        print("usage: find-offset-refs <hex_offset> [segment_prefix]")
        return
    target_off = int(args[0], 16)
    seg_prefix = args[1] if len(args) > 1 else None

    listing = program.getListing()
    fm = program.getFunctionManager()
    seen_funcs = {}
    count = 0
    max_hits = 200

    # Match pattern "+ 0x4fc" or "+ 0x4fe" in textual rendering
    needle = f"0x{target_off:x}"
    needle_alt = f"{target_off:#x}"

    for ins in listing.getInstructions(True):
        addr = ins.getAddress()
        addr_hex = f"0x{addr}"
        if seg_prefix and not addr_hex.startswith(seg_prefix):
            continue
        ins_text = str(ins)
        if needle in ins_text.lower():
            func = fm.getFunctionContaining(addr)
            fn_ep = f"0x{func.getEntryPoint()}" if func else "(no fn)"
            fn_nm = func.getName() if func else "?"
            key = fn_ep
            seen_funcs.setdefault(key, (fn_nm, []))[1].append((addr_hex, ins_text))
            count += 1
            if count >= max_hits:
                break

    print(f"Found {count} instructions referencing offset 0x{target_off:x}")
    print(f"Across {len(seen_funcs)} functions:\n")
    for fn_ep, (fn_nm, hits) in list(seen_funcs.items())[:40]:
        print(f"  {fn_ep}  {fn_nm}  ({len(hits)} hits)")
        for addr_hex, ins_text in hits[:3]:
            print(f"    {addr_hex}  {ins_text}")
