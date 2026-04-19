#!/usr/bin/env python3
"""
ghidra_query · 通过 pyghidra 查询已分析的 FlexColor.dll
用法: ./run.sh <command> [args...]
"""
import argparse
import os
import sys
from pathlib import Path

# Make 'scripts' package importable
sys.path.insert(0, str(Path(__file__).parent))

import pyghidra
pyghidra.start()  # Must happen before any `ghidra.*` imports

from scripts import common
from scripts import list_classes
from scripts import list_methods
from scripts import decompile
from scripts import dump_pipeline
from scripts import find_luts
from scripts import vtable
from scripts import probe
from scripts import probe2
from scripts import probe3
from scripts import run_rtti
from scripts import read_const
from scripts import disasm
from scripts import class_methods_all
from scripts import find_u16_luts
from scripts import find_str_xrefs
from scripts import dump_xml_registry
from scripts import list_rtti_classes
from scripts import find_offset_refs

COMMANDS = {
    "list-classes":  list_classes,
    "list-methods":  list_methods,
    "decompile":     decompile,
    "dump-pipeline": dump_pipeline,
    "find-luts":     find_luts,
    "vtable":        vtable,
    "probe":         probe,
    "probe2":        probe2,
    "probe3":        probe3,
    "run-rtti":      run_rtti,
    "read-const":    read_const,
    "disasm":        disasm,
    "class-methods-all": class_methods_all,
    "find-u16-luts": find_u16_luts,
    "find-str-xrefs": find_str_xrefs,
    "dump-xml-registry": dump_xml_registry,
    "list-rtti-classes": list_rtti_classes,
    "find-offset-refs": find_offset_refs,
}


def main():
    parser = argparse.ArgumentParser(description="Ghidra query tool for FlexColor RE")
    parser.add_argument("command", choices=COMMANDS.keys(), help="查询类型")
    parser.add_argument("args", nargs=argparse.REMAINDER, help="子命令参数")
    parser.add_argument("--program", default=os.environ.get("GHIDRA_PROGRAM", "FlexColor.dll"))
    args = parser.parse_args()

    # run-rtti 需要可写模式
    writable_cmds = {"run-rtti"}
    readonly = args.command not in writable_cmds
    project = common.open_project(readonly=readonly)
    try:
        program = common.open_program(project, args.program, readonly=readonly)
        mod = COMMANDS[args.command]
        mod.run(program, args.args)
        if not readonly:
            project.save(program)
    finally:
        project.close()


if __name__ == "__main__":
    main()
