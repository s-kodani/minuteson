#!/usr/bin/env python3
"""Deterministic IPC registration and role grants; builds never rewrite sources."""
import argparse
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
_RAW = re.compile(r'(?:br|cr|r)(#{0,255})"')
_CHARACTER = re.compile(r"(?:b)?'(?:\\(?:x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f_]{1,8}\}|.)|[^'\\])'")
_TOKEN = re.compile(r"r#[A-Za-z_]\w*|'[A-Za-z_]\w*|[A-Za-z_]\w*|\d+|::|->")


# This deliberately recognizes a constrained Rust command declaration grammar.
# Unknown header syntax fails closed; native compilation remains the authority
# for full Rust/type correctness. Comments/literals cannot forge a declaration.
def rust_tokens(source):
    out, index = [], 0
    while index < len(source):
        char = source[index]
        if char.isspace():
            index += 1
            continue
        if source.startswith("//", index):
            end = source.find("\n", index)
            index = len(source) if end < 0 else end
            continue
        if source.startswith("/*", index):
            index += 2
            depth = 1
            while depth:
                if index >= len(source):
                    raise ValueError("unterminated Rust comment")
                if source.startswith("/*", index):
                    depth += 1
                    index += 2
                elif source.startswith("*/", index):
                    depth -= 1
                    index += 2
                else:
                    index += 1
            continue
        raw = _RAW.match(source, index)
        if raw:
            end = source.find('"' + raw[1], raw.end())
            if end < 0:
                raise ValueError("unterminated raw Rust string")
            end += 1 + len(raw[1])
            out.append(source[index:end])
            index = end
            continue
        quoted = char == '"' or char in 'bc' and source[index:index + 2].endswith('"')
        character = _CHARACTER.match(source, index)
        if character:
            out.append(character[0])
            index = character.end()
            continue
        if quoted:
            end = index + (1 if char == '"' else 2)
            while end < len(source):
                if source[end] == "\\":
                    end += 2
                elif source[end] == '"':
                    end += 1
                    break
                else:
                    end += 1
            else:
                raise ValueError("unterminated Rust string")
            out.append(source[index:end])
            index = end
            continue
        token = _TOKEN.match(source, index)
        if token:
            out.append(token[0])
            index = token.end()
        else:
            out.append(char)
            index += 1
    return out


def group_end(tokens, index, opening="(", closing=")"):
    if tokens[index] != opening:
        raise ValueError("invalid Rust declaration delimiter")
    depth = 0
    for cursor in range(index, len(tokens)):
        if tokens[cursor] == opening:
            depth += 1
        elif tokens[cursor] == closing:
            depth -= 1
            if depth == 0:
                return cursor
    raise ValueError("unterminated Rust declaration group")


def split_top(tokens, separator=","):
    pairs = {"(": ")", "[": "]", "{": "}", "<": ">"}
    stack, result, start = [], [], 0
    for index, token in enumerate(tokens):
        if token in pairs:
            stack.append(pairs[token])
        elif token in pairs.values():
            if not stack or stack.pop() != token:
                raise ValueError("unbalanced Rust declaration type")
        elif token == separator and not stack:
            result.append(tokens[start:index])
            start = index + 1
    if stack:
        raise ValueError("unbalanced Rust declaration type")
    if start < len(tokens):
        result.append(tokens[start:])
    return result


def rust_type(tokens):
    if not tokens:
        raise ValueError("missing Rust command type")
    # Removing whitespace alone would conflate `dyn Trait` with `dynTrait`.
    result = ""
    previous = ""
    for token in tokens:
        if previous and (previous[-1].isalnum() or previous[-1] == "_") and (token[0].isalnum() or token[0] == "_"):
            result += " "
        result += token
        previous = token
    return result


def generic_type(tokens):
    if "<" not in tokens:
        return rust_type(tokens), []
    start = tokens.index("<")
    end = group_end(tokens, start, "<", ">")
    if end != len(tokens) - 1:
        return rust_type(tokens), []
    return rust_type(tokens[:start]), split_top(tokens[start + 1:end])


def injected_argument(tokens):
    while tokens and (tokens[0] in ("&", "mut") or tokens[0].startswith("'")):
        tokens = tokens[1:]
    base, _ = generic_type(tokens)
    return base in {"State", "tauri::State", "AppHandle", "tauri::AppHandle",
                    "Window", "tauri::Window", "Webview", "tauri::Webview",
                    "WebviewWindow", "tauri::WebviewWindow", "tauri::ipc::Request"}


def wire_name(name, casing):
    name = name.removeprefix("r#")
    if casing == "snake_case":
        return name
    parts = name.split("_")
    return parts[0] + "".join(p[:1].upper() + p[1:] for p in parts[1:])


def command_signatures(source):
    tokens, found, index = rust_tokens(source), {}, 0
    while index + 4 < len(tokens):
        if tokens[index:index + 4] != ["#", "[", "tauri", "::"] or tokens[index + 4] != "command":
            index += 1
            continue
        end = group_end(tokens, index + 1, "[", "]")
        options = tokens[index + 5:end]
        casing = "camelCase"
        if options:
            if options[:1] != ["("] or options[-1:] != [")"]:
                raise ValueError("unsupported tauri command attribute")
            for option in split_top(options[1:-1]):
                if option == ["async"]:
                    continue
                if len(option) == 3 and option[:2] == ["rename_all", "="] and option[2] in ('"camelCase"', '"snake_case"'):
                    casing = json.loads(option[2])
                else:
                    raise ValueError("unsupported tauri command attribute")
        cursor = end + 1
        # Other attributes between command and fn do not change the wire shape.
        while tokens[cursor:cursor + 2] == ["#", "["]:
            cursor = group_end(tokens, cursor + 1, "[", "]") + 1
        if tokens[cursor] == "pub":
            cursor += 1
            if tokens[cursor] == "(":
                cursor = group_end(tokens, cursor) + 1
        asynchronous = tokens[cursor] == "async"
        if asynchronous:
            cursor += 1
        if tokens[cursor] != "fn" or not re.fullmatch(r"[A-Za-z_]\w*", tokens[cursor + 1]):
            raise ValueError("unsupported tauri command function declaration")
        name = tokens[cursor + 1]
        cursor += 2
        if tokens[cursor] != "(":
            raise ValueError("generic tauri command headers require explicit generator support")
        end_args = group_end(tokens, cursor)
        arguments = []
        for argument in split_top(tokens[cursor + 1:end_args]):
            if len(argument) > 1 and argument[0] == "mut":
                argument = argument[1:]
            if len(argument) < 3 or not re.fullmatch(r"(?:r#)?[A-Za-z_]\w*", argument[0]) or argument[1] != ":":
                raise ValueError(f"unsupported command argument pattern: {name}")
            arg_name, arg_type = argument[0], argument[2:]
            if injected_argument(arg_type):
                continue
            base, _ = generic_type(arg_type)
            arguments.append({"name": arg_name, "wireName": wire_name(arg_name, casing),
                              "type": rust_type(arg_type), "optional": base in ("Option", "std::option::Option")})
        cursor = end_args + 1
        if tokens[cursor] == "->":
            cursor += 1
            start, stack = cursor, []
            while cursor < len(tokens):
                token = tokens[cursor]
                if not stack and token in ("{", "where", ";"):
                    break
                if token in ("<", "(", "["):
                    stack.append({"<": ">", "(": ")", "[": "]"}[token])
                elif token in (">", ")", "]"):
                    if not stack or stack.pop() != token:
                        raise ValueError("unbalanced command response")
                cursor += 1
            response = tokens[start:cursor]
        else:
            response = ["(", ")"]
        if tokens[cursor] != "{":
            raise ValueError(f"unsupported command response clause: {name}")
        base, parameters = generic_type(response)
        output = {"type": rust_type(response), "ok": rust_type(response), "error": None}
        if base in ("Result", "std::result::Result"):
            if len(parameters) != 2:
                raise ValueError(f"command Result needs explicit success and error types: {name}")
            output.update(ok=rust_type(parameters[0]), error=rust_type(parameters[1]))
        if name in found or len({a["wireName"] for a in arguments}) != len(arguments):
            raise ValueError(f"duplicate command declaration or wire argument: {name}")
        found[name] = {"async": asynchronous, "arguments": arguments, "response": output}
        index = group_end(tokens, cursor, "{", "}") + 1
    return found


def registry_commands(root=ROOT, synchronize=False):
    native = root / "app/src-tauri"
    registry = json.loads((native / "command_registry.json").read_text(encoding="utf8"))
    roles = ("main", "recorder", "recorder-info", "recording-confirmation")
    commands = registry["commands"]
    names = [c["name"] for c in commands]
    if registry["schemaVersion"] != 1 or len(names) != len(set(names)):
        raise ValueError("invalid command registry")
    cache, registered = {}, set()
    for command in commands:
        if command["handler"].split("::")[-1] != command["name"] or not command["roles"] or any(r not in roles for r in command["roles"]):
            raise ValueError("invalid handler or role")
        parts = command["handler"].split("::")
        relative = Path(*parts[:-1]) if len(parts) > 1 else Path("lib")
        path = native / "src" / relative.with_suffix(".rs")
        if not path.exists():
            path = native / "src" / relative / "mod.rs"
        if path not in cache:
            cache[path] = command_signatures(path.read_text(encoding="utf8"))
        signature = cache[path].get(command["name"])
        if signature is None:
            raise ValueError(f"registered handler lacks #[tauri::command]: {command['handler']}")
        if synchronize:
            command["signature"] = signature
        elif command.get("signature") != signature:
            raise ValueError(f"command signature differs: {command['handler']}; review and run tools/mon_contracts.py --sync-signatures")
        registered.add((path, command["name"]))
    # Every declared native command needs an explicit role and wire contract.
    for path in sorted((native / "src").rglob("*.rs")):
        if path not in cache:
            cache[path] = command_signatures(path.read_text(encoding="utf8"))
        for name in cache[path]:
            if (path, name) not in registered:
                raise ValueError(f"native command absent from registry: {path.relative_to(native)}::{name}")
    if synchronize:
        (native / "command_registry.json").write_text(json.dumps(registry, ensure_ascii=False, indent=2) + "\n", encoding="utf8", newline="\n")
    return commands


def typescript_type(value):
    tokens = rust_tokens(value)
    base, parameters = generic_type(tokens)
    if base in ("String", "str", "&str"):
        return "string"
    if base == "bool":
        return "boolean"
    if base in ("u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "usize", "isize", "f32", "f64"):
        return "number"
    if base in ("Option", "std::option::Option") and len(parameters) == 1:
        return f"({typescript_type(rust_type(parameters[0]))}) | null"
    if base in ("Vec", "std::vec::Vec") and len(parameters) == 1:
        return f"Array<{typescript_type(rust_type(parameters[0]))}>"
    # Domain DTO shapes have separate generated schemas or native validation.
    return "unknown"


def typescript_commands(commands):
    result = "// Generated by tools/mon_contracts.py from verified Rust signatures. Do not edit.\n"
    result += "export interface CommandArguments {\n"
    for command in commands:
        arguments = command["signature"]["arguments"]
        members = "; ".join(json.dumps(a["wireName"]) + ("?" if a["optional"] else "") + ": " + typescript_type(a["type"]) for a in arguments)
        result += f'  {json.dumps(command["name"])}: ' + ("{ " + members + " }" if arguments else "Record<string, never>") + ";\n"
    result += "}\nexport type CommandName = keyof CommandArguments;\n"
    result += "export type CommandArgs<C extends CommandName> = CommandArguments[C];\n"
    result += "export const commandContracts = " + json.dumps({c["name"]: c["signature"] for c in commands}, ensure_ascii=False, indent=2) + " as const;\n"
    return result


def products(root=ROOT):
    native = root / "app/src-tauri"
    commands = registry_commands(root)
    roles = ("main", "recorder", "recorder-info", "recording-confirmation")
    names = [c["name"] for c in commands]
    header = "// Generated by tools/mon_contracts.py; edit command_registry.json.\n"
    manifest = header + "#[rustfmt::skip]\npub const COMMANDS: &[&str] = &[\n" + "".join(f'    "{c}",\n' for c in names) + "];\n"
    handler = header + "tauri::generate_handler![\n" + "".join(f'    {c["handler"]},\n' for c in commands) + "]\n"
    permissions = "# Generated by tools/mon_contracts.py; native state checks remain authoritative.\n"
    for role in roles:
        permissions += f'\n[[permission]]\nidentifier = "{role}-commands"\ndescription = "Commands for {role}."\ncommands.allow = [\n'
        permissions += "".join(f'  "{c["name"]}",\n' for c in commands if role in c["roles"])
        permissions += "]\n"
    grants = header + "#[rustfmt::skip]\npub fn permits(command: &str, role: &str) -> bool {\n    match command {\n"
    for c in commands:
        grants += f'        "{c["name"]}" => matches!(role, ' + ' | '.join(json.dumps(r) for r in c["roles"]) + "),\n"
    grants += "        _ => false,\n    }\n}\n"
    return {native / "app_commands.rs": manifest, native / "src/command_handler.rs": handler,
            native / "src/command_roles.rs": grants, native / "permissions/application.toml": permissions,
            root / "app/src/generated/commands.ts": typescript_commands(commands)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--sync-signatures", action="store_true", help="Explicitly review/update the registry signature snapshots from Rust")
    args = parser.parse_args()
    if args.sync_signatures:
        registry_commands(synchronize=True)
    for path, content in products().items():
        if args.check:
            if not path.exists() or path.read_text(encoding="utf8") != content:
                raise SystemExit(f"Stale generated contract: {path.relative_to(ROOT)}")
        else:
            path.write_text(content, encoding="utf8", newline="\n")


if __name__ == "__main__":
    main()
