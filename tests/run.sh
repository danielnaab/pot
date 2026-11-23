#!/usr/bin/env bash
# End-to-end test of pot: test roots built with pot.lib.mkRoot, whose grants file and state are in a work
# directory, and workspaces whose kits come from fixture bundles (flakes that call pot.lib.pot.kitsFor
# themselves). Sections: contracts (the contracts' schemas and fixtures), kits (kitsFor and the kits' own checks, as a
# producer's build shows them), pot (runs, under bubblewrap; pot_seatbelt on macOS), pi (the pi kit), dispatch
# (clone mode), sibling (wrapped kit commands and siblings), rollback (graft v2.21.0's pot reads what this pot
# wrote). Needs nix, git and jq; fetches nixpkgs on the first run. Runs need unprivileged user namespaces;
# without them those parts are skipped, saying so.
# Usage: tests/run.sh [SECTION...] (from anywhere; default: every section but rollback, in order). KEEP=1 leaves
# the work directory, to debug a failure.
set -euo pipefail

src=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap '[ -n "${KEEP:-}" ] || rm -rf "$work"' EXIT
[ -z "${KEEP:-}" ] || echo "work: $work"
# pot as roots and producers use it: a copy of this checkout's working tree, unchanged, used as a path: input
pot=$work/pot
mkdir -p "$pot" && (cd "$src" && tar --exclude=./.git --exclude=./result -cf - .) | tar -C "$pot" -xf -
# git as a reader's would run, with what tests need: file:// submodules (git >= 2.38 refuses them by
# default), no signing, a name. Through the environment, so pot's own git calls get it too.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_CONFIG_COUNT=6 \
  GIT_CONFIG_KEY_0=protocol.file.allow GIT_CONFIG_VALUE_0=always \
  GIT_CONFIG_KEY_1=user.name GIT_CONFIG_VALUE_1=test \
  GIT_CONFIG_KEY_2=user.email GIT_CONFIG_VALUE_2=test@example.com \
  GIT_CONFIG_KEY_3=commit.gpgsign GIT_CONFIG_VALUE_3=false \
  GIT_CONFIG_KEY_4=tag.gpgsign GIT_CONFIG_VALUE_4=false \
  GIT_CONFIG_KEY_5=init.defaultBranch GIT_CONFIG_VALUE_5=main
git_() { git "$@"; }
# tools the test runs from nixpkgs, at the commit pot's flake.lock pins (already fetched for pot)
# shellcheck disable=SC2034 # used by later sections
np="github:NixOS/nixpkgs/$(jq -r '.nodes.nixpkgs.locked.rev' "$pot/flake.lock")"
sys=$(nix eval --impure --raw --expr builtins.currentSystem)
pass() { echo "ok   $*"; }
fail() { echo "FAIL $*" >&2; [ -z "${out:-}" ] || { echo "(last output:)" >&2; tail -8 <<<"$out" >&2; }; exit 1; }
# expect <description> <command...>: the command must succeed
expect() { local d=$1; shift; if "$@"; then pass "$d"; else fail "$d"; fi; }
# refuse <description> <command...>: the command must fail
refuse() { local d=$1; shift; if "$@"; then fail "$d"; else pass "$d"; fi; }
quiet() { "$@" >/dev/null 2>&1; }
# ran <description> <command...>: the command must succeed; its output (both streams) in $out
ran() { local d=$1; shift; if out=$("$@" 2>&1); then pass "$d"; else echo "$out" >&2; fail "$d"; fi; }
# refused <description> <command...>: the command must fail; its output (both streams) in $out
refused() { local d=$1; shift; if out=$("$@" 2>&1); then echo "$out" >&2; fail "$d"; else pass "$d"; fi; }
# has TEXT: $out holds TEXT as a whole line; holds TEXT: anywhere
has() { grep -qxF -- "$1" <<<"$out"; }
holds() { grep -qF -- "$1" <<<"$out"; }
# holdsj TEXT: TEXT in $out with wrapped lines (continuations indented 4) joined again (pot's long exit lines)
holdsj() { grep -qF -- "$1" <<<"$(awk 'NR > 1 && /^    [^ ]/ { sub(/^    /, " "); printf "%s", $0; next } { if (NR > 1) print ""; printf "%s", $0 } END { print "" }' <<<"$out")"; }
# jsv ARGS: check-jsonschema (the contracts' validator), from the nixpkgs pot's flake.lock pins, built once
jsv_bin=""
jsv() {
  [ -n "$jsv_bin" ] || jsv_bin=$(nix build --no-link --print-out-paths --impure \
    --expr "(builtins.getFlake \"path:$pot\").inputs.nixpkgs.legacyPackages.$sys.check-jsonschema")/bin/check-jsonschema
  "$jsv_bin" "$@"
}
# py ARGS: python3 from the same nixpkgs (pot's own code, read by the contracts section)
py_bin=""
py() {
  [ -n "$py_bin" ] || py_bin=$(nix build --no-link --print-out-paths --impure \
    --expr "(builtins.getFlake \"path:$pot\").inputs.nixpkgs.legacyPackages.$sys.python3")/bin/python3
  "$py_bin" -I "$@"
}
# valid SCHEMA [FILE...]: each FILE (default: $out, one JSON document) validates against SCHEMA, a path in the checkout
valid() {
  local s=$1 r; shift
  if [ $# -eq 0 ]; then printf '%s\n' "$out" > "$work/valid.json"; set -- "$work/valid.json"; fi
  r=$(jsv --default-filetype json --schemafile "$src/$s" "$@" 2>&1) || { echo "$r" >&2; return 1; }
}
# valid_lines SCHEMA FILE...: each line of each FILE (JSON Lines) validates against SCHEMA
valid_lines() {
  local s=$1 f n=0 d; shift
  d=$(mktemp -d -p "$work")
  for f in "$@"; do while IFS= read -r l; do n=$((n + 1)); printf '%s\n' "$l" > "$d/$n.json"; done < "$f"; done
  [ "$n" -gt 0 ] || { echo "valid_lines: no lines in $*" >&2; return 1; }
  valid "$s" "$d"/*.json
}
# stdout_of CMD...: CMD's stdout alone (its stderr in $work/stderr), for ran: a JSON document to parse
stdout_of() { "$@" 2>"$work/stderr"; }
# exit_of CMD...: CMD's exit status, printed (its output in $work/exit.out)
exit_of() { local rc=0; "$@" >"$work/exit.out" 2>&1 || rc=$?; echo "$rc"; }

# Providers: git repositories at release tags, for submodule kits. prov NAME: a work tree $work/src/NAME, with
# its origin $work/NAME.git (its URL file://$work/NAME.git); release NAME TAG [-a]: commit everything, tag
# (annotated with -a), push.
url() { echo "file://$work/$1.git"; }
prov() {
  mkdir -p "$work/src/$1"
  git_ -C "$work/src/$1" init -q
  git_ init -q --bare "$work/$1.git"
  git_ -C "$work/src/$1" remote add origin "$(url "$1")"
  echo "# $1" > "$work/src/$1/README.md"
}
release() {
  local d=$work/src/$1
  git_ -C "$d" add -A && git_ -C "$d" commit -q --allow-empty -m "$2"
  if [ "${3:-}" = -a ]; then git_ -C "$d" tag -a -m "$1 $2" "$2"; else git_ -C "$d" tag "$2"; fi
  git_ -C "$d" push -q origin HEAD:refs/heads/main "refs/tags/$2"
}
sha() { git_ -C "$work/src/$1" rev-parse "$2^{commit}"; }
# by_hand NAME [TAG [DIR]]: NAME at TAG (default v1.0.0) as a git submodule in DIR (default scions) of the repo in
# the current directory, with plain git; staged
by_hand() {
  local d=${3:-scions}
  git_ submodule add -q --force -- "$(url "$1")" "$d/$1" 2>/dev/null
  git_ -C "$d/$1" checkout -q --detach "${2:-v1.0.0}"
  [ -f "$d/README.md" ] || echo "Submodules at release tags." > "$d/README.md"
  git_ add -- "$d/$1" "$d/README.md" .gitmodules
}
commit() { git_ add -A && git_ commit -q --allow-empty -m "${1:-commit}"; }

# The contracts' schemas are JSON Schema 2020-12, and their golden fixtures (what readers test against) validate.
# What pot really prints is validated where it prints it: pot, dispatch, sibling, pi.
sec_contracts() {
  # runs/1: pot runs --json, and a log's lines
  local rs=contracts/runs-1.schema.json es=contracts/runs-1.event.schema.json fx=$src/contracts/fixtures/runs-1 f
  expect "contracts: runs/1's schemas are 2020-12 schemas" quiet jsv --check-metaschema "$src/$rs" "$src/$es"
  expect "contracts: runs/1's record and list fixtures validate" valid "$rs" "$fx"/*.json
  expect "contracts: runs/1's log fixtures validate, line by line" valid_lines "$es" "$fx"/*.log.jsonl
  # each record fixture is what pot's own code (core/runs.py) computes from its log, as pot runs does (a
  # stopped run's grantedSince: false, as load_record adds it when no grant covers it since)
  for f in "$fx"/*.log.jsonl; do
    out=$(py -c 'import json, os, sys; sys.path.insert(0, sys.argv[1]); import runs as R
rid = os.path.basename(sys.argv[2])[:4]; rec = R.record(rid, R.read_log(sys.argv[2]), lambda s: False)
if rec["state"] == "stopped" and rec["stop"]: rec["stop"]["grantedSince"] = False
print(json.dumps(rec))' "$src/core" "$f")
    expect "contracts: runs/1's $(basename "$f" .log.jsonl).record.json is pot's record of its log" \
      test "$(jq -cS . <<<"$out")" = "$(jq -cS . "${f%.log.jsonl}.record.json")"
  done
  expect "contracts: runs/1's list.json holds those records, newest first" \
    test "$(jq -cS '.runs' "$fx/list.json")" = "$(jq -cS -s 'sort_by(.started) | reverse' "$fx"/[0-9a-f][0-9a-f][0-9a-f][0-9a-f].record.json)"
  out=$(jq '.state = "review" | .clone = null' "$fx/41c7.record.json")
  refuse "contracts: runs/1 refuses a review with no clone" quiet valid "$rs"
  out=$(jq '.runs[0].owner = "ada"' "$fx/list.json")
  refuse "contracts: runs/1 refuses a record field it does not define" quiet valid "$rs"
  printf '%s\n' '{"ev":"exit","by":"host","time":"2026-11-20T10:00:00"}' > "$work/bad.jsonl"
  refuse "contracts: runs/1 refuses an exit event with no status" quiet valid_lines "$es" "$work/bad.jsonl"
  # trailer/1: the Pot-Run: trailer, read with git's own trailer parser; pot trailer writes each fixture
  local ts=contracts/trailer-1.schema.json tx=$src/contracts/fixtures/trailer-1 v
  expect "contracts: trailer/1's schema is a 2020-12 schema" quiet jsv --check-metaschema "$src/$ts"
  for f in "$tx"/*.txt; do
    out=$(git_ interpret-trailers --parse < "$f" | sed -n 's/^Pot-Run: //p' | jq -R . | jq -cs .)
    expect "contracts: trailer/1's $(basename "$f"): git's parser gives values.json's values" \
      test "$out" = "$(jq -c --arg f "$(basename "$f")" '.messages[$f]' "$tx/values.json")"
    for v in $(jq -r '.[]' <<<"$out"); do
      out=$(jq -n --arg v "$v" '$v')
      expect "its value $v validates" valid "$ts"
    done
  done
  out='"7f3a2"' && refuse "contracts: trailer/1 refuses a longer id" quiet valid "$ts"
  for f in "$tx"/message.txt "$tx"/with-sign-off.txt; do
    v=$(git_ interpret-trailers --parse < "$f" | sed -n 's/^Pot-Run: //p')
    grep -v '^Pot-Run: ' "$f" | awk '{ a[NR] = $0 } END { n = NR; while (n > 0 && a[n] == "") n--; for (i = 1; i <= n; i++) print a[i] }' > "$work/trailer.msg"
    POT_RUN=$v py -c 'import sys; sys.path.insert(0, sys.argv[1]); import pot; sys.exit(pot.trailer([sys.argv[2], "message"]))' "$src/core" "$work/trailer.msg"
    expect "contracts: pot trailer writes $(basename "$f") from its message" cmp -s "$f" "$work/trailer.msg"
  done
  # policy.json/1: what pot writes for a backend (each run's policy.json, pot explain --json)
  local ps=contracts/policy-1.schema.json px=$src/contracts/fixtures/policy-1
  expect "contracts: policy.json/1's schema is a 2020-12 schema" quiet jsv --check-metaschema "$src/$ps"
  expect "contracts: policy.json/1's fixtures validate" valid "$ps" "$px"/*.json
  expect "contracts: policy.json/1's fixtures hold a clone, a sibling row, an exec row, bindings, a proxy, inspect and Seatbelt" quiet \
    jq -se 'any(.[]; .mode == "clone") and any(.[].children[]; .how == "sibling") and any(.[].children[]; .how == "exec")
      and any(.[]; .bindings | length > 0) and any(.[].items[]; .kind == "network" and .class == "reach" and .effective)
      and any(.[].items[]; .kind == "view") and any(.[]; .backend.name == "seatbelt")' "$px"/*.json
  expect "contracts: policy.json/1 has invalid fixtures" test -n "$(find "$px/invalid" -name '*.json')"
  for f in "$px"/invalid/*.json; do
    refuse "contracts: policy.json/1 refuses invalid/$(basename "$f")" quiet valid "$ps" "$f"
  done
  # launch.json/1: the launcher's facts pot writes for a backend beside policy.json (and the keys a backend adds)
  local ls=contracts/launch-1.schema.json lx=$src/contracts/fixtures/launch-1
  expect "contracts: launch.json/1's schema is a 2020-12 schema" quiet jsv --check-metaschema "$src/$ls"
  expect "contracts: launch.json/1's fixtures validate" valid "$ls" "$lx"/*.json
  expect "contracts: launch.json/1's fixtures hold a clone, a root repository, a sibling, a hook's variables, an explain, the nix store, inspect's walk, a hidden path git tracks and the pins" quiet \
    jq -se 'any(.[]; .clone) and any(.[]; .rootRepo) and any(.[]; .siblingOf) and any(.[]; .hookEnv.GIT_INDEX_FILE)
      and any(.[]; has("policyDigest") | not) and any(.[]; .closureCount | type == "string") and any(.[]; .walk)
      and any(.[]; .hiddenTracked) and any(.[].pins[]?; (.placeholders | length > 0) and (.sealed | length > 0))' "$lx"/*.json
  for f in "$px"/*.json; do
    [ "$(basename "$f")" = seatbelt.json ] && continue     # composed by compose(), with no run: no launch.json
    expect "contracts: launch.json/1 has policy.json/1's $(basename "$f")'s launch.json" test -f "$lx/$(basename "$f")"
  done
  expect "contracts: launch.json/1 has invalid fixtures" test -n "$(find "$lx/invalid" -name '*.json')"
  for f in "$lx"/invalid/*.json; do
    refuse "contracts: launch.json/1 refuses invalid/$(basename "$f")" quiet valid "$ls" "$f"
  done
}
# runs_valid STATE CMD...: every line of every run log under STATE, and CMD's `runs --json --all`, validate against
# runs/1's schemas (contracts/runs-1*.schema.json): what pot really wrote, in each section that runs it
runs_valid() {
  local st=$1 logs id; shift
  logs=$(find "$st" -name log.jsonl -size +0 | sort)
  [ -n "$logs" ] || { echo "runs_valid: no logs under $st" >&2; return 1; }
  # shellcheck disable=SC2086 # one path per line, no spaces (the tests' own folders)
  valid_lines contracts/runs-1.event.schema.json $logs || return 1
  out=$("$@" runs --json --all 2>/dev/null) && valid contracts/runs-1.schema.json || return 1
  id=$(jq -r '.runs[0].id // empty' <<<"$out")
  [ -n "$id" ] || { echo "runs_valid: pot runs --json --all listed no run" >&2; return 1; }
  out=$("$@" runs "$id" --json 2>/dev/null) && valid contracts/runs-1.schema.json
}

# policies_valid STATE: every policy.json pot wrote under STATE (each run's, a sibling's, a 125's) validates against
# policy.json/1's schema (contracts/policy-1.schema.json): what pot really wrote, in each section that runs it
policies_valid() {
  local ps
  ps=$(find "$1" -name policy.json -size +0 | sort)
  [ -n "$ps" ] || { echo "policies_valid: no policy.json under $1" >&2; return 1; }
  # shellcheck disable=SC2086 # one path per line, no spaces (the tests' own folders)
  valid contracts/policy-1.schema.json $ps
}

# launches_valid STATE: every launch.json under STATE (each run's, after its backend's prepare) validates against
# launch.json/1's schema (contracts/launch-1.schema.json): what pot and its backend really wrote, as policies_valid
launches_valid() {
  local ls
  ls=$(find "$1" -name launch.json -size +0 | sort)
  [ -n "$ls" ] || { echo "launches_valid: no launch.json under $1" >&2; return 1; }
  # shellcheck disable=SC2086 # one path per line, no spaces (the tests' own folders)
  valid contracts/launch-1.schema.json $ls
}

# kits: pot's side of the seam with a producer (pot.lib.pot.kitsFor, contract pot-lib/1): the bundle it builds
# (pot-kits/1), its kits.just text and the kits' own checks, as a producer's build shows them, from a fixture that
# calls kitsFor directly; and the kits' and roots' messages for forms that are gone. A producer's own side (which
# kits it hands over, where it writes kits.just) is the producer's to test (tuteur's tests take pot as an input).
sec_kits() {
  local ex="let p = builtins.getFlake \"path:$pot\"; pkgs = p.inputs.nixpkgs.legacyPackages.$sys; in"
  # kf ARGS: a bundle, built from kitsFor's arguments (Nix text), its output path in $out
  # shellcheck disable=SC2317 # run through ran and refused
  kf() { nix build --no-link --print-out-paths --impure --expr "$ex p.lib.pot.kitsFor { inherit pkgs; $1 }"; }
  # shellcheck disable=SC2016 # Nix text, not shell
  local tool='(pkgs.writeShellScriptBin "kk-tool" "echo kk-tool")'
  ran "kits: kitsFor builds a bundle" kf "self = { kit = { protect = [ \"AGENTS.md\" ]; tools = [ pkgs.hello ]; }; file = \"flake.nix (fixture)\"; };
    scions.kk = { file = \"scions/kk/kit.nix\"; kit = { protect = [ \"kk.yaml\" ]; requests.network = [ \"api.kk.example\" ];
      commands.kk-tool.program = \"\${$tool}/bin/kk-tool\"; recipes.kk = \"kk-tool --all\"; }; };
    tools = [ $tool ]; files = [ \"**/producer.nix\" \"scions\" ];
    recipes.mine = { run = \"hello -g hi\"; description = \"The producer's own\"; };"
  local b && b=$(tail -1 <<<"$out")
  ran "its kit.json" jq -c . "$b/kit.json"
  expect "is pot-kits/1: version, self, scions, protect" test "$(jq -c 'keys' "$b/kit.json")" = '["protect","scions","self","version"]'
  expect "holds the repo kit, with its file" holds '"self":{"file":"flake.nix (fixture)","protect":["AGENTS.md"],"tools":["/nix/store/'
  expect "and the submodule kit" holds '"kk":{"commands":{"kk-tool":{"program":"/nix/store/'
  expect "naming its file" holds '"file":"scions/kk/kit.nix"'
  expect "and the producer's files to protect" holds '"protect":["**/producer.nix","scions"]'
  expect "its bin/ holds the tools" test -x "$b/bin/hello"
  expect "kits: bin/kk-tool is the command's wrapper" test "$(readlink "$b/bin/kk-tool")" = "$b/wrappers/bin/kk-tool"
  expect "with its manifest" test "$(jq -c . "$b/commands/kk-tool.json")" = '{"command":"kk-tool","kit":"scions.kk","version":1}'
  expect "the wrapper says whose it is" grep -qx '# pot (pot-kits/1): kk-tool, a command of scions.kk, run by pot under its own policy' "$b/wrappers/bin/kk-tool"
  expect "the bundle has its closure" grep -q hello "$b/closure"
  # kits.just: pot's fixed header (pot-kits/1; a producer compares these bytes), then each recipe by name
  ran "kits: kits.just" cat "$b/kits.just"
  expect "its header names no producer" test "$(head -2 "$b/kits.just")" = "# Generated from the kits' recipes (pot-kits/1) by the bundle's producer, which writes and compares it:
# do not edit. The importing justfile defines \`tools\`, the tools package (its root, not its bin/)."
  expect "has the kit's recipe, its words quoted" holds 'exec "{{tools}}/bin/kk-tool" '"'"'--all'"'"' "$@"'
  expect "and the producer's own" holds 'exec "{{tools}}/bin/hello" '"'"'-g'"'"' '"'"'hi'"'"' "$@"'
  expect "with its description" has "# The producer's own"
  ran "kits: passthru.kitsJust is the same text, without a build" nix eval --impure --raw --expr "$ex (p.lib.pot.kitsFor { inherit pkgs;
    scions.kk = { file = \"scions/kk/kit.nix\"; kit = { commands.kk-tool.program = \"/bin/true\"; recipes.kk = \"kk-tool --all\"; }; };
    recipes.mine = { run = \"hello -g hi\"; description = \"The producer's own\"; }; }).passthru.kitsJust"
  expect "byte for byte" test "$out" = "$(cat "$b/kits.just")"
  ran "kits: passthru.wrappers" nix build --no-link --print-out-paths --impure --expr "$ex (p.lib.pot.kitsFor { inherit pkgs;
    scions.kk = { file = \"f\"; kit.commands.kk-tool.program = \"/bin/true\"; }; }).passthru.wrappers"
  expect "holds the wrapper alone" test "$(ls "$(tail -1 <<<"$out")/bin")" = kk-tool
  # the bundle's checks, each naming the kit's file
  refused "kits: a malformed recipe" nix eval --impure --raw --expr "$ex (p.lib.pot.kitsFor { inherit pkgs; recipes.x = { shell = 1; }; }).drvPath"
  expect "is named" holds 'pot: kitsFor: recipes.x is "<command> <args>" or { run; description; }'
  refused "kits: a kit with an unknown field" kf 'scions.kkbad = { file = "scions/kkbad/kit.nix"; kit.paths = [ ]; };'
  expect "pot's check names the field and the file" holds "pot: scions/kkbad/kit.nix: paths is not a kit field"
  refused "kits: a unit that is not { kit; file; }" kf 'scions.x = { kit = { }; };'
  expect "is named" holds "pot: kitsFor: scions.x is { kit; file; } (file: a string naming where the kit is written)"
  refused "kits: two kits with one command name" nix eval --impure --raw --expr "$ex (p.lib.pot.kitsFor { inherit pkgs;
    self = { kit.commands.x.program = \"/bin/true\"; file = \"f\"; }; scions.a = { kit.commands.x.program = \"/bin/true\"; file = \"f\"; }; }).drvPath"
  expect "is named" holds "pot: kitsFor: two kits ship a command named x (self and scions.a): a wrapper has one name"
  # kit.packages is gone at the source (graft v2.17.0): kitsFor refuses it, naming tools (pot still reads it in a
  # bundle that graft v2.16.x built: sec_pot)
  refused "kits: a submodule kit's kit.packages" kf 'scions.kkpkg = { file = "scions/kkpkg/kit.nix"; kit.packages = [ pkgs.hello ]; };'
  expect "names tools" holds "pot: scions/kkpkg/kit.nix: packages is tools' old name, gone in v2.17.0: give tools"
  refused "kits: the repo kit's kit.packages" kf 'self = { file = "flake.nix (perSystem)"; kit.packages = [ pkgs.hello ]; };'
  expect "names tools" holds "pot: flake.nix (perSystem): packages is tools' old name, gone in v2.17.0: give tools"
  # the other v2.16.0 forms, gone: each error names its replacement, in pot's names
  refused "kits: mkRoot's defaultAccount" nix eval --impure --raw --expr "$ex (p.lib.mkRoot { inherit pkgs; kits.\"claude-code@a\" = p.kits.claude-code { inherit pkgs; }; defaultAccount = \"a\"; }).drvPath"
  expect "names defaultInstance" holds 'pot.lib.mkRoot: defaultAccount is gone (v2.17.0): defaultInstance.<kit> = "NAME"'
  refused "kits: claude-code's account" nix eval --impure --json --expr "$ex (p.kits.claude-code { inherit pkgs; account = \"a\"; }).version"
  expect "names the kits key" holds "pot.kits.claude-code: account is gone (v2.17.0): the instance is the root's kits key (\"claude-code@NAME\")"
  refused "kits: claude-code's provider and region" nix eval --impure --json --expr "$ex (p.kits.claude-code { inherit pkgs; provider = \"bedrock\"; region = \"us-east-1\"; }).version"
  expect "names api" holds 'pot.kits.claude-code: provider and region are gone (v2.17.0): api = pot.apis.bedrock { region = "…"; } (or pot.apis.anthropic)'
  # pot.apis.gateway takes an https url on 443 (the kit reaches its host there)
  ran "kits: pot.apis.gateway" nix eval --impure --raw --expr "$ex (p.kits.claude-code { inherit pkgs; api = p.apis.gateway { url = \"https://llm.example.test/\"; }; }).env.set.ANTHROPIC_BASE_URL"
  expect "sets the base URL under /anthropic" test "$out" = "https://llm.example.test/anthropic"
  refused "kits: a gateway url that is not https" nix eval --impure --json --expr "$ex (p.apis.gateway { url = \"http://llm.example.test\"; }).url"
  expect "names the form" holds 'pot.apis.gateway: url = "http://llm.example.test": give https://HOST[/PATH] (pot reaches HOST on port 443)'
  refused "kits: a gateway url on another port" nix eval --impure --json --expr "$ex (p.apis.gateway { url = \"https://llm.example.test:8443\"; }).url"
  expect "is refused the same way" holds 'pot.apis.gateway: url = "https://llm.example.test:8443": give https://HOST[/PATH] (pot reaches HOST on port 443)'
  refused "kits: a root kit's packages" nix eval --impure --raw --expr "$ex (p.lib.mkRoot { inherit pkgs; kits.x.packages = [ pkgs.hello ]; }).drvPath"
  expect "names tools" holds "pot: the root (programs.pot.kits.x): packages is tools' old name, gone in v2.17.0: give tools"
  ran "kits: programs.pot.defaultAccount (home-manager fails on the assertion)" nix eval --impure --raw --expr "$ex builtins.concatStringsSep \"\\n\" (map (a: a.message) (builtins.filter (a: !a.assertion) (p.inputs.nixpkgs.lib.evalModules { modules = [ p.homeManagerModules.default ({ lib, ... }: {
      options.home.packages = lib.mkOption { default = [ ]; }; options.home.file = lib.mkOption { default = { }; };
      options.assertions = lib.mkOption { default = [ ]; };
      config = { _module.args.pkgs = pkgs; programs.pot.enable = true; programs.pot.defaultAccount = \"a\"; }; }) ]; }).config.assertions))"
  expect "names defaultInstance" holds 'no longer has any effect; please remove it.'
  expect "and its replacement" holds 'Use programs.pot.defaultInstance.<kit> = "NAME" (v2.17.0).'
}

# pot: pot.lib.mkRoot, and runs under a test root whose grants file and state are in the work
# directory, in a workspace whose kits come from a fixture bundle: a flake that calls pot.lib.pot.kitsFor
# itself. Composition's rules have unit tests (tests/test_*.py, from nix flake check); here, what a
# run does. Runs need unprivileged user namespaces; without them that part is skipped, saying so.
sec_pot() {
  # sbk: a submodule at a release tag; the fixture flake gives its kit (pot reads only the bundle)
  prov sbk && release sbk v1.0.0
  local c=$work/sbc && mkdir -p "$c" && git_ -C "$c" init -q && cd "$c"
  cat > flake.nix <<EOF
{
  inputs.pot.url = "path:$pot";
  outputs = { pot, ... }: {
    packages = pot.inputs.nixpkgs.lib.genAttrs [ "$sys" ] (s: let pkgs = pot.inputs.nixpkgs.legacyPackages.\${s};
        # v2.20.0: fail and sleep, for pot_seatbelt's siblings
        tool = pkgs.writeShellScriptBin "sbk-tool" ''
          case "''\${1:-}" in
            fail) echo "sbk-tool: failing"; exit 7 ;;
            sleep) trap 'echo "sbk-tool: INT"; exit 130' INT; echo "sbk-tool: sleeping"; \${pkgs.coreutils}/bin/sleep "\$2" & wait; exit 0 ;;
            noterm) trap "" TERM; \${pkgs.coreutils}/bin/sleep "\$2" & wait; exit 0 ;;
          esac
          echo sbk-tool hi
        ''; in {
      pot-kits = pot.lib.pot.kitsFor {
        inherit pkgs;
        self = { kit = { protect = [ "AGENTS.md" ]; tools = [ pkgs.hello ]; }; file = "flake.nix (fixture)"; };
        scions.sbk = { file = "kits/sbk.nix"; kit = { requests.network = [ "api.sbk.example" ]; protect = [ "sbk.yaml" ];
          commands.sbk-tool.program = "\${tool}/bin/sbk-tool"; }; };
        tools = [ tool ];
        files = [ "**/kit-owner.nix" "scions" ];
      };
    });
  };
}
EOF
  echo "# agents" > AGENTS.md && echo "a: 1" > sbk.yaml && echo "default:" > justfile && echo data > notes.md
  by_hand sbk && commit init && quiet nix flake lock && commit lock

  # pot.lib.mkRoot: a test root
  local r=$work/sbroot && mkdir -p "$r" "$work/sbrootrepo" "$work/sbhidden"
  git_ -C "$work/sbrootrepo" init -q && git_ -C "$work/sbrootrepo" commit -q --allow-empty -m init
  cat > "$r/flake.nix" <<EOF
{
  inputs.pot.url = "path:$pot";
  inputs.nixpkgs.follows = "pot/nixpkgs";
  outputs = { pot, nixpkgs, ... }: {
    packages = nixpkgs.lib.genAttrs [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ] (s: {
      # a root with no agent kit: the marker floor still applies (R2-2)
      pot0 = pot.lib.mkRoot { pkgs = nixpkgs.legacyPackages.\${s}; name = "plain";
        grantsFile = "$work/sbgrants0/grants.json"; stateDir = "$work/sbstate0"; };
      pot = pot.lib.mkRoot {
        pkgs = nixpkgs.legacyPackages.\${s};
        name = "test";
        grantsFile = "$work/sbgrants/grants.json";
        stateDir = "$work/sbstate";
        rootRepos = [ "$work/sbrootrepo" "fake.example/rootx" ];
        # S3: a granted write request that names a root repository
        kits.rootwriter.commands.rwtest = { program = "\${nixpkgs.legacyPackages.\${s}.bashInteractive}/bin/bash";
          requests.write = [ "$work/sbrootrepo" ]; };
        hidden = [ "$work/sbhidden" ];
        kits.daniel.requests.network = [ "nothing.invalid:1" ];
        # relay: a command kit with state, a reasserted file, and the status.relay, credential and mcp bindings
        kits.relay = let pkgs = nixpkgs.legacyPackages.\${s}; in {
          # kit 1.1: its transcripts, read as the claude-code kit reads Claude's
          version = "1.1";
          agent.resume = { transcripts = "~/.st/projects/**/*.jsonl"; args = [ "--resume" "{id}" ];
            render = (pot.kits.claude-code { inherit pkgs; }).agent.resume.render; };
          files."~/.st/conf.json" = { source = pkgs.writeText "conf.json" (builtins.toJSON { a = 1; projects."*".t = true; });
            mode = "reassert"; keys = [ "a" "projects.*.t" ]; };
          commands.relaytest = { program = "\${pkgs.bashInteractive}/bin/bash"; requests = {
            state = [ { name = "st"; at = "~/.st"; } ];
            capabilities = [ { name = "status.relay"; } { name = "credential"; host = "api.example"; env = "TESTTOK"; }
              { name = "mcp"; server = "s"; } ]; }; };
        };
        kits.fetcher.commands.fetchtest = { program = "\${nixpkgs.legacyPackages.\${s}.bashInteractive}/bin/bash";
          requests.capabilities = [ { name = "git.fetch"; remote = "ssh://git@fake.example/*"; } ]; };
        kits.fixed.commands.fixedtest = { program = "\${nixpkgs.legacyPackages.\${s}.bashInteractive}/bin/bash";
          fixedDirs = { "~/.claude/plugins" = "$work/cfg/plugins"; "~/.claude/skills" = "$work/cfg/skills"; };
          realDirs = [ "~/.claude/plugins" ]; skillDirs = [ "synced" ];
          memory = { at = "~/.claude/projects"; host = "$work/cfg/projects"; };
          requests.state = [ { name = "claude"; at = "~/.claude"; } ]; };
        kits."claude-code@t" = pot.kits.claude-code { pkgs = nixpkgs.legacyPackages.\${s};
          configDir = ".pot-test-no-such-config"; plugins = false;
          binary = nixpkgs.legacyPackages.\${s}.writeShellScriptBin "claude" (builtins.readFile ./stub-claude.sh); };
        kits."pair@a".requests.read = [ "/tmp" ];
        kits."pair@b".requests.read = [ "/var" ];
        kits.nixer.commands.nixcheck = { program = "\${nixpkgs.legacyPackages.\${s}.bashInteractive}/bin/bash"; requests.capabilities = [ { name = "nix"; } ]; };
        bindings = {
          "status.relay"."*" = { workmux = true; };
          credential."api.example" = { via = "file"; from.file = "$work/sbtok"; };
          mcp.s = { url = "https://mcp.example/mcp"; via = "env"; env = "MCPKEY"; from.file = "$work/sbmcp.env"; };
          nix."*" = { via = "daemon-untrusted"; };
          "git.fetch"."ssh://git@fake.example/*" = { broker = "git"; key.agent = true; };
        };
        # instances chosen by the caller's variables (agent.select.env): the most specific match wins
        kits."sel@a" = { version = "1.1"; agent.select.env.SELV = "a"; commands.seltest.program = "\${nixpkgs.legacyPackages.\${s}.bashInteractive}/bin/bash"; };
        kits."sel@b" = { version = "1.1"; agent.select.env.SELV = "b"; commands.seltest.program = "\${nixpkgs.legacyPackages.\${s}.bashInteractive}/bin/bash"; };
        kits."sel@c" = { version = "1.1"; agent.select.env.SELV = "b"; commands.seltest.program = "\${nixpkgs.legacyPackages.\${s}.bashInteractive}/bin/bash"; };
        kits."sel@bw" = { version = "1.1"; agent.select.env = { SELV = "b"; SELW = "$work"; }; commands.seltest.program = "\${nixpkgs.legacyPackages.\${s}.bashInteractive}/bin/bash"; };
        defaultInstance.sel = "a";
        contexts.work.include = [ "daniel" ];
        contexts.readonly = { extends = "work"; access = "read"; };
        # 0.2.0: access none, no workspace
        contexts.blind = { extends = "work"; access = "none"; };
        contexts.blindclone = { extends = "dispatch"; access = "none"; };
        contexts.blindview = { extends = "inspect"; access = "none"; };
        inspect.home = [ ];
      };
    });
  };
}
EOF
  # the stub claude: the overlay's SessionStart, PreToolUse, a refused request, and PostToolUseFailure, as
  # Claude runs them for a failed Bash call
  cat > "$r/stub-claude.sh" <<'STUB'
o=$2
hook() { jq -r --arg e "$1" '.hooks[$e][0].hooks[0].command' "$o"; }
"$(hook SessionStart)" </dev/null
echo '{"hook_event_name":"PreToolUse"}' | "$(hook PreToolUse)"
curl -s -o /dev/null https://denied-tool.example/ || true
echo '{"hook_event_name":"PostToolUseFailure"}' | "$(hook PostToolUseFailure)"
STUB
  (cd "$r" && quiet nix flake lock)
  ran "pot: mkRoot builds a root" nix build -o "$work/sbr" "path:$r#packages.$sys.pot"
  ran "pot: and a root with no agent kit" nix build -o "$work/sbr0" "path:$r#packages.$sys.pot0"
  ran "pot: on aarch64-darwin, the root evaluates" nix eval --raw "path:$r#packages.aarch64-darwin.pot.drvPath"
  ran "and it is darwin's" nix eval --json "path:$r#packages.aarch64-darwin.pot.passthru.root.darwin"
  expect "(its backend is Seatbelt)" has "true"
  ran "pot: the default root" nix eval --raw "path:$pot#packages.$sys.default.drvPath"
  expect "is packages.pot too" test "$out" = "$(nix eval --raw "path:$pot#packages.$sys.pot.drvPath")"
  ran "pot: apps.default" nix eval --raw "path:$pot#apps.$sys.default.program"
  expect "runs pot" holds "/bin/pot"
  ran "pot: apps.pot" nix eval --raw "path:$pot#apps.$sys.pot.program"
  expect "is the same" holds "/bin/pot"

  # macOS: the Seatbelt backend's runs (pot_seatbelt); the rest of this section is bubblewrap's
  if [ "$(uname)" = Darwin ]; then
    pot_seatbelt "$c"
    return 0
  fi
  local bw && bw=$(nix build --no-link --print-out-paths "$np#bubblewrap")/bin/bwrap
  if ! "$bw" --unshare-all --unshare-user --disable-userns --ro-bind / / true 2>/dev/null; then
    echo "skip pot: runs (bubblewrap can't create an unprivileged user namespace here)"
    return 0
  fi
  # sb_ ARGS: the test root's pot, quiet, never as an agent (the marker would refuse the prompt);
  # sb_loud: with its startup and exit lines
  # shellcheck disable=SC2317 # run through ran, expect and refuse
  sb_loud() { env -u CLAUDECODE -u CODEX_SANDBOX -u POT "$work/sbr/bin/pot" "$@"; }
  # shellcheck disable=SC2317
  sb_() { POT_QUIET=1 sb_loud "$@"; }
  # tty_ ANSWERS CMD: CMD on a terminal, its prompts answered from ANSWERS (one line each)
  # shellcheck disable=SC2317
  tty_() { local a=$1; shift; (sleep 2; printf '%s' "$a") | script -qec "env -u CLAUDECODE -u CODEX_SANDBOX -u POT $*" /dev/null; }

  refused "pot: never granted, no terminal" sb_loud true
  expect "stops with the diff" holds "pot (work): daniel (root:daniel) requests reach that is not granted:"
  expect "each item" holds "  + network nothing.invalid:1"
  expect "and the fix" holds "  grant it: pot grant daniel"
  sb_ true >/dev/null 2>&1 && st=0 || st=$?
  expect "exit 125" test "$st" = 125
  refused "pot: pot grant under an agent marker" env CLAUDECODE=1 "$work/sbr/bin/pot" grant
  expect "is refused (the claude-code kit's agent.markers)" holds "pot grant is for people: CLAUDECODE is set"
  # the marker floor holds on a root with no agent kit (R2-2)
  refused "pot: pot grant under CLAUDECODE, a root with no agent kit" env CLAUDECODE=1 "$work/sbr0/bin/pot" grant
  expect "is refused (the floor)" holds "pot grant is for people: CLAUDECODE is set"
  refused "pot: pot grant under the root's agentMarkers" env -u CLAUDECODE CODEX_SANDBOX=1 "$work/sbr/bin/pot" grant
  expect "is refused (CODEX_SANDBOX, the default)" holds "pot grant is for people: CODEX_SANDBOX is set"

  # instances: CMD@NAME, --instance (--account its old name), the caller's variables, the default
  ran "pot: an instance, the default" env -u SELV -u SELW POT_QUIET=1 "$work/sbr/bin/pot" explain seltest
  expect "is sel@a" holds "run        seltest -> root:sel@a"
  ran "pot: an instance by the caller's variable" env SELV=b SELW="$work/." POT_QUIET=1 "$work/sbr/bin/pot" explain seltest
  expect "the most specific (a path, compared by its real path)" holds "run        seltest -> root:sel@bw"
  refused "pot: two equal matches" env -u SELW SELV=b POT_QUIET=1 "$work/sbr/bin/pot" explain seltest
  expect "stop, naming them" holds "pot: seltest: the caller's variables match sel@b and sel@c equally: choose one with seltest@NAME or --instance NAME"
  ran "pot: CMD@NAME first" env SELV=b POT_QUIET=1 "$work/sbr/bin/pot" explain seltest@c
  expect "is sel@c" holds "run        seltest -> root:sel@c"
  ran "pot: --instance" env SELV=b POT_QUIET=1 "$work/sbr/bin/pot" explain --instance b seltest
  expect "is sel@b" holds "run        seltest -> root:sel@b"
  refused "pot: --account, gone in v2.17.0" env -u SELV POT_QUIET=1 "$work/sbr/bin/pot" explain --account c seltest
  expect "names --instance and CMD@NAME" holds "pot: --account is gone (v2.17.0): --instance NAME, or CMD@NAME"
  refused "pot: an instance the root does not declare" sb_ explain seltest@nope
  expect "names them" holds "pot: seltest@nope: this root declares sel@a, sel@b, sel@bw, sel@c"
  refused "pot: CMD@NAME and another --instance" sb_ explain --instance a seltest@c
  expect "is usage" holds "seltest@c and --instance a name two instances"
  ran "pot: the inline first-grant prompt, at a terminal" tty_ $'y\n' "$work/sbr/bin/pot" echo ran-after-grant
  expect "asks" holds "daniel   root kit (root:daniel), for every run that uses it   not granted"
  expect "ordinary items together" holds "grant these? [y/N]"
  expect "and the run continues" holds "ran-after-grant"
  ran "the grants file" jq -c '.grants["root:daniel"].requests' "$work/sbgrants/grants.json"
  expect "holds the approved set" has '["network nothing.invalid:1"]'
  ran "pot: pot grant KIT, nothing new" sb_ grant daniel
  expect "says so" has "pot grant: daniel is granted"
  refused "pot: pot grant with no terminal" sb_ grant relay
  expect "is refused" holds "pot grant is for people: it reads its answers from /dev/tty, and there is none"

  ran "pot: a command runs, in the repository" sb_loud pwd
  expect "there" has "$(pwd -P)"
  expect "with a startup line" holds "pot: work · sbc · pot explain"
  # shellcheck disable=SC2016 # the run's shell expands them
  ran "pot: the run's environment" sb_ bash -c 'echo "ctx=$POT home=$HOME tools=${POT_TOOLS:+set}"; ls -A ~; hello; cat /run/pot/policy.json | jq -r .context; head -1 /run/pot/README.md'
  expect "POT is the context" holds "ctx=work home="
  expect "the repo tools" holds "tools=set"
  expect "HOME holds the hidden list and the gitconfig" holds ".pot-hidden"
  expect "the repo kit's package" holds "Hello, world!"
  expect "policy.json" has "work"
  expect "the README" has "# pot: this run (work)"
  ran "pot: ~/.pot-hidden" sb_ bash -c 'cat ~/.pot-hidden'
  expect "lists the root's hidden paths" has "$work/sbhidden"
  # shellcheck disable=SC2088 # pot prints ~ literally
  expect "and pot's defaults" has "~/.ssh"
  ran "pot: the caller's variables" env SB_DECOY=decoy-secret POT_QUIET=1 "$work/sbr/bin/pot" bash -c 'env | grep -c decoy-secret; tr "\0" " " </proc/1/cmdline'
  expect "none of them" has "0"
  refuse "and no host path on bwrap's command line" holds "$work/sbstate"

  # protections
  for p in justfile AGENTS.md sbk.yaml flake.nix .gitmodules .git/config; do
    refuse "pot: read-only: $p" sb_ bash -c "echo x >> $p" 2>/dev/null
  done
  expect "pot: the repository is writable" sb_ bash -c 'echo y >> notes.md'
  refuse "pot: .git can't be renamed" sb_ mv .git .git2 2>/dev/null
  refuse "pot: nor a scion's folder" sb_ mv scions/sbk scions/sbk2 2>/dev/null
  ran "pot: a commit inside" sb_ bash -c 'git add notes.md && git commit -qm inside && git log -1 --format=%s'
  expect "is made" has "inside"
  # v2.21.0 (R0.5): the run's gitconfig runs pot trailer, which writes POT_RUN into the message
  # shellcheck disable=SC2016 # the run's shell expands it
  ran "pot: POT_RUN, and a commit's trailer" sb_ bash -c 'echo "run=$POT_RUN"; git commit -q --allow-empty -m trailer && git log -1 --format="%(trailers:key=Pot-Run,valueonly)"'
  expect "POT_RUN is the run's id" grep -qE '^run=[0-9a-f]{4}$' <<<"$out"
  # git's config-based hooks are 2.54 and later: an older git in the run ignores the hook (no trailer, no failure)
  # shellcheck disable=SC2016 # the run's shell expands them
  if sb_ bash -c 'v=$(git --version | cut -d" " -f3); [ "$(printf "%s\n2.54\n" "$v" | sort -V | head -1)" = 2.54 ]'; then
    expect "the commit carries it" test "$(sed -n 's/^run=//p' <<<"$out")" = "$(tail -1 <<<"$out")"
  else
    expect "an older git: no trailer, and the commit is made" test -z "$(tail -1 <<<"$out" | grep -v '^run=')"
  fi
  # pot host: POT_RUN, the after look, the log's after event
  git_ config hook.pot-trailer.event prepare-commit-msg && git_ config hook.pot-trailer.command "$work/sbr/bin/pot trailer"
  # shellcheck disable=SC2016 # the command's shell expands it
  ran "pot: pot host, a commit" sb_loud host bash -c 'git -c commit.gpgsign=false commit -q --allow-empty -m "host side" && echo "run=$POT_RUN"'
  git_ config --unset hook.pot-trailer.event && git_ config --unset hook.pot-trailer.command
  local hrid && hrid=$(sed -n 's/^run=//p' <<<"$out")
  expect "the after look names the commit" holds "pot: host run $hrid · 1 commit on HEAD"
  expect "with its status and subject" grep -qE "^  [0-9a-f]{9} N -  host side$" <<<"$out"
  if [ "$(printf '%s\n2.54\n' "$(git_ --version | cut -d' ' -f3)" | sort -V | head -1)" = 2.54 ]; then
    expect "the commit carries the run" test "$(git_ log -1 --format='%(trailers:key=Pot-Run,valueonly)')" = "$hrid"
  fi
  ran "pot: pot runs, a host run" sb_ runs "$hrid" --json
  expect "its record lists the commit" test "$(jq -r '.commits[0].subject + " " + .commits[0].status + " " + .mode' <<<"$out")" = "host side N none"
  ran "pot: a nested repository, made inside" sb_loud bash -c 'mkdir evil && git -C evil init -q'
  expect "is reported after the run" holdsj "pot: WARNING: the run created what host git, a shell prompt, an editor or direnv reads, and may run code from: evil/.git"
  rm -rf -- "$c/evil"
  ln AGENTS.md "$work/pot-agents-link"
  refused "pot: a protected file with another hard link" sb_ true
  expect "stops the run" holds "pot (work): AGENTS.md has another hard link"
  rm -f -- "$work/pot-agents-link"
  ln -s "$work" linked && sed -i 's|protect = \[ "AGENTS.md" \]|protect = [ "AGENTS.md" "linked/x" ]|' flake.nix && commit "a link"
  refused "pot: a link on a protected path" sb_ true
  expect "stops the run" holds "pot (work): linked is a symbolic link, so the sandbox can't pin linked/x, below it"
  git_ rm -q linked && sed -i 's| "linked/x"||' flake.nix && commit "no link"

  # hidden, and reach flags
  refused "pot: --read of a hidden path" sb_ --read "$work/sbhidden" true
  expect "is refused" holds "pot (work): --read $work/sbhidden: which resolves into a hidden path"
  refused "pot: --read of pot's state" sb_ --read "$work/sbstate" true
  expect "is refused" holds "which is pot's own state"
  mkdir -p "$work/sbextra" && echo extra > "$work/sbextra/f"
  ran "pot: --read" sb_loud --read "$work/sbextra" cat "$work/sbextra/f"
  expect "reads it" has "extra"
  expect "and the exit line names the permanent fix" holds "pot: --read $work/sbextra was for this run."
  expect "on its own line" holds "  permanent: requests.read in the repo kit (flake.nix, fixture)"
  expect "and the other" holds "             or kits.daniel.requests.read, for every repository (rebuild the root)"
  refuse "pot: without --read, it is not there" sb_ cat "$work/sbextra/f" 2>/dev/null

  # network: through the proxy, to granted hosts alone; denials reach the run and the exit line
  # shellcheck disable=SC2016 # the run's shell expands them
  local ask='exec 3<>/dev/tcp/127.0.0.1/3128 && printf "CONNECT %s HTTP/1.1\r\nHost: %s\r\n\r\n" "$1" "$1" >&3 && head -1 <&3'
  ran "pot: a host not granted" sb_loud bash -c "$ask; cat /run/pot/denials" _ denied.example:443
  expect "the proxy refuses it" holds "HTTP/1.1 403 Forbidden (pot: not in this run)"
  expect "the run reads the denial, with its fix" holds "pot (work): the run tried denied.example:443 (not in this run)."
  expect "one run" holds "  one run:   pot --net denied.example …"
  expect "permanent" holds "  permanent: requests.network in the repo kit (flake.nix, fixture)"
  expect "and the other" holds "             or kits.daniel.requests.network, for every repository (rebuild the root)"
  expect "and the exit line says so, line by line" test "$(grep -c -e '^pot (work): the run tried denied.example:443 (not in this run).$' -e '^  one run:   pot --net denied.example …$' <<<"$out")" -ge 4
  expect "with the run id" holds "pot: run "
  ran "pot: a granted host (nothing listens: Bad Gateway)" sb_ bash -c "$ask" _ nothing.invalid:1
  expect "passes the proxy" holds "HTTP/1.1 502 Bad Gateway"
  # F1 (v2.19.0): the host's loopback is never reached, even granted: a literal, localhost., and the host's own
  # name (which resolves to loopback on platform-vm), with a listener there that must see no connection
  local lp lpid hn
  # it ends by itself (a failed check exits before the kill) and holds none of the test's output
  python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); s.listen(); s.settimeout(300); print(s.getsockname()[1], flush=True)
while True:
    c, _ = s.accept(); print("CONNECTED", flush=True); c.close()' > "$work/lo-port" 2>/dev/null </dev/null &
  lpid=$!
  for _ in $(seq 50); do [ -s "$work/lo-port" ] && break; sleep 0.1; done
  lp=$(head -1 "$work/lo-port")
  hn=$(hostname)
  local h
  for h in "127.0.0.1:$lp" "localhost.:$lp" "$hn:$lp"; do
    if [ "${h%%:*}" = "$hn" ] && ! getent ahosts "$hn" | awk '{print $1}' | grep -qE '^(127\.|::1$)'; then
      echo "skip pot: $hn does not resolve to loopback here"; continue
    fi
    ran "pot: F1, a granted host at the host's loopback ($h)" sb_loud --net "$h" bash -c "$ask; cat /run/pot/denials" _ "$h"
    expect "the proxy refuses it" holds "HTTP/1.1 403 Forbidden (pot: the host's loopback)"
    expect "the run reads why" holds "pot (work): the run tried $h, the host's loopback ("
    expect "and that no grant reaches it" holds "a run cannot reach it (host-loopback is reserved for a later release)."
    refuse "with no --net fix (none applies)" holds "one run:   pot --net ${h%%:*}"
  done
  kill "$lpid" 2>/dev/null || true
  refuse "pot: F1, nothing reached the listener" grep -q CONNECTED "$work/lo-port"
  ran "pot: the readonly context" sb_loud readonly bash -c 'echo z >> notes.md 2>&1; true'
  expect "has no write" holds "Read-only file system"
  # 0.2.0: access none is no workspace at all: neither the repository nor its git directory is in the run, which
  # starts in its home; a folder a read request binds above the workspace shows it empty
  # shellcheck disable=SC2016 # the run's shell expands them
  ran "pot: access none" sb_ blind bash -c 'echo "pwd=$PWD"; test -e "$1" && echo "workspace seen"; test -e "$1/.git" && echo "git seen"
    grep -c "There is no workspace" /run/pot/README.md; pot why "$1/notes.md"; true' _ "$c"
  expect "starts in the run's home" has "pwd=$HOME"
  refuse "the workspace is not there" holds "workspace seen"
  refuse "nor its git directory" holds "git seen"
  expect "the run's README says so" has "1"
  expect "pot why says so" holds "/notes.md is in the workspace repo:"
  expect "(not seen)" holds "which this run does not see (access none)"
  ran "pot: explain, access none" sb_ explain blind
  expect "names no workspace" has "limits     no workspace (access none): the run does not see it, and starts in its home"
  expect "and the workspace row" holds "(none: not in the run) $c"
  ran "pot: explain --json, access none" sb_ explain --json blind
  expect "has access none, nothing protected or sealed" test "$(jq -c '[.access, [.workspace[] | .protected, .sealed]]' <<<"$out")" = '["none",[[],[]]]'
  mkdir -p "$work/sbup/ws" && git_ -C "$work/sbup/ws" init -q && echo secret-ish > "$work/sbup/ws/f" && echo seen > "$work/sbup/g"
  git_ -C "$work/sbup/ws" add f && git_ -C "$work/sbup/ws" commit -qm init
  cd "$work/sbup/ws"
  # shellcheck disable=SC2016 # the run's shell expands them
  ran "pot: access none, under a folder --read binds" sb_ blind --read "$work/sbup" bash -c 'cat "$1/g"; echo "ws=[$(ls -A "$1/ws")]"; true' _ "$work/sbup"
  expect "the folder is read" has "seen"
  expect "the workspace in it is empty" has "ws=[]"
  # shellcheck disable=SC2016 # the run's shell expands them
  ran "pot: access none, inspect's view" sb_ blindview bash -c 'echo "pwd=$PWD"; test -e "$1/f" && echo "workspace seen"; true' _ "$work/sbup/ws"
  expect "starts in the home" has "pwd=$HOME"
  refuse "and has no workspace" holds "workspace seen"
  cd "$c"
  refused "pot: access none in a clone" sb_ blindclone true
  expect "is refused" has "pot (blindclone): access none: the run sees no workspace, so it has no clone to work in; a context with access none runs in place"
  ran "pot: a log per run" bash -c "cat '$work'/sbstate/runs/*/log.jsonl"
  expect "start" holds '"ev":"start","by":"host"'
  expect "after" holds '"ev":"after","by":"host"'

  # bindings: status.relay through a workmux shim to the launching pane, live denials through tmux,
  # a credential and an MCP key by file, the nix relay. Host workmux and tmux are fakes that log.
  local fb=$work/sbfake && mkdir -p "$fb"
  # shellcheck disable=SC2016 # literal text
  printf '#!/bin/sh\necho "$TMUX_PANE $*" >> %s/workmux.log\n' "$work" > "$fb/workmux"
  printf '#!/bin/sh\necho "$*" >> %s/tmux.log\n' "$work" > "$fb/tmux"
  chmod +x "$fb/workmux" "$fb/tmux"
  # B1 (v2.17.1): binding files are checked before the grant prompts and the clone. relay is not granted yet,
  # and its credential's and MCP key's files are not made yet
  refused "pot: B1, a run whose binding files are missing" sb_ relaytest true
  expect "B1: names the file, the binding and the fix" has "pot (work): $work/sbtok is missing (the credential binding api.example): create it, with mode 600, then start again"
  expect "B1: and the MCP key's" has "pot (work): $work/sbmcp.env is missing (the mcp binding s): create it, with mode 600, then start again"
  refuse "B1: before the grant check" holds "pot grant relay"
  expect "B1: exits 1" test "$(sb_ relaytest true >/dev/null 2>&1; echo $?)" = 1
  refused "pot: B1, at a terminal" tty_ $'y\ny\ny\n' "$work/sbr/bin/pot" relaytest true
  expect "B1: refused there too" holds "$work/sbtok is missing (the credential binding api.example)"
  refuse "B1: with no grant prompt" holds "+ capability"
  refuse "B1: and no grant" grep -q '"root:relay"' "$work/sbgrants/grants.json"
  refused "pot: B1, a dispatch" sb_ dispatch relaytest true
  expect "B1: names its context" has "pot (dispatch): $work/sbtok is missing (the credential binding api.example): create it, with mode 600, then start again"
  expect "B1: leaves no clone" test -z "$(for d in "$work"/sbstate/runs/*/; do [ -d "$d/clone" ] && [ ! -s "$d/log.jsonl" ] && echo "$d"; done)"
  expect "B1: and no run folder" test -z "$(for d in "$work"/sbstate/runs/*/; do [ -e "$d/log.jsonl" ] || echo "$d"; done)"
  ran "pot: B1, explain does not check the files" sb_ explain relaytest
  out=$(sb_ why TESTTOK 2>&1 || true)
  refuse "B1: nor does why" holds "is missing (the"
  printf 'MCPKEY=mcp-value\n' > "$work/sbmcp.env" && printf 'tok-value\n' > "$work/sbtok" && chmod 000 "$work/sbtok"
  if [ ! -r "$work/sbtok" ]; then
    refused "pot: B1, an unreadable binding file" sb_ relaytest true
    expect "B1: is named" has "pot (work): $work/sbtok is unreadable (the credential binding api.example): create it, with mode 600, then start again"
  fi
  chmod 600 "$work/sbtok"
  printf 'tok-value\n' > "$work/sbtok" && printf 'MCPKEY=mcp-value\n' > "$work/sbmcp.env"
  ran "pot: pot grant relay, at a terminal" tty_ $'y\ny\ny\n' "$work/sbr/bin/pot" grant relay
  expect "ordinary items together" holds "+ capability status.relay  bound: workmux relay"
  expect "a powerful item alone" holds "powerful: + capability credential api.example env=TESTTOK  bound: file $work/sbtok, as a file in /run/pot/secrets that its variable names (the run holds it)"
  ran "pot: explain --backend-command relaytest" sb_ explain --backend-command relaytest
  # 0.2.0: via file, the variable names the run's file
  expect "the credential's variable names its file" holds "--setenv TESTTOK /run/pot/secrets/TESTTOK"
  expect "an MCP key by env is held, not shown" holds "--setenv MCPKEY '(the run holds it)'"
  refuse "its value never shows" holds "tok-value"
  # shellcheck disable=SC2016 # the run's shell expands them
  ran "pot: a run with bindings" env PATH="$fb:$PATH" TMUX=/tmp/fake,1,0 TMUX_PANE=%9 POT_QUIET=1 "$work/sbr/bin/pot" relaytest -c \
    'echo "tok=$TESTTOK value=$(cat "$TESTTOK") mode=$(stat -c %a "$TESTTOK") mine=$(stat -c %u "$TESTTOK")=$(id -u)";
     echo "secrets=[$(ls -A /run/pot/secrets)] mcp=$MCPKEY"; echo x > "$TESTTOK" 2>/dev/null || echo "file read-only";
     jq -c . /run/pot/mcp.json; workmux set-window-status working; workmux rm x; jq -c . ~/.st/conf.json;
     mkdir -p ~/.st/projects/p && echo "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello from the run\"}}" > ~/.st/projects/p/t.jsonl;
     exec 3<>/dev/tcp/127.0.0.1/3128 && printf "CONNECT live.example:443 HTTP/1.1\r\n\r\n" >&3 && head -1 <&3' 2>&1
  # 0.2.0: via file is honoured: the value in a file of the run's own (mode 400, the run's user), its variable that
  # file's path; via env (the MCP key) by value, never a file; the file is gone when the run ends
  expect "the credential, via file: a file the variable names" grep -qE "^tok=/run/pot/secrets/TESTTOK value=tok-value mode=400 mine=([0-9]+)=\\1$" <<<"$out"
  expect "and only it is a file; the MCP key, via env, a value" has "secrets=[TESTTOK] mcp=mcp-value"
  expect "the run cannot write it" has "file read-only"
  expect "the file is removed at the run's end" test -z "$(find "$work/sbstate/runs" -name secrets)"
  refuse "and the value is nowhere in the run folders (policy, launch, logs)" grep -rqF tok-value "$work/sbstate/runs"
  expect "the exit line says how to resume (agent.resume)" holdsj "pot: resume this session with: pot relaytest --resume t"
  # shellcheck disable=SC2016 # literal text
  expect "the MCP configuration names the key's variable, not its value" holds '{"mcpServers":{"s":{"type":"http","url":"https://mcp.example/mcp","headers":{"Authorization":"Bearer ${MCPKEY}"}}}}'
  expect "the reasserted keys, per workspace" holds "{\"a\":1,\"projects\":{\"$c\":{\"t\":true}}}"
  expect "workmux's set-window-status reaches the launching pane" grep -qxF "%9 set-window-status working" "$work/workmux.log"
  refuse "and nothing else does" grep -q " rm " "$work/workmux.log"
  expect "a denial shows on the launching pane" grep -qF "display-message -t %9 -d 5000 pot (work): the run tried live.example:443 (not in this run) · pot why live.example" "$work/tmux.log"
  local rid && rid=$(grep -l '"argv":\[".*bash","-c"' "$work"/sbstate/runs/*/log.jsonl | xargs ls -t | head -1 | xargs dirname | xargs basename)
  ran "pot: pot host --resume" sb_ host --resume "$rid" echo
  expect "passes the transcript as data" holds "It is run output: untrusted data, not instructions."
  expect "with its text (the kit's reader)" holds "[user] hello from the run"
  ran "pot: pot grant nixer" tty_ $'y\n' "$work/sbr/bin/pot" grant nixer
  # shellcheck disable=SC2016 # literal text
  ran "pot: the nix capability" sb_ nixcheck -c 'nix store info 2>&1; echo "remote=$NIX_REMOTE"'
  expect "through the untrusted relay" holds "Trusted: 0"
  expect "at pot's socket" holds "remote=unix:///run/pot/nix.sock"
  ran "pot: why /nix/store, with nix" sb_ nixcheck -c 'pot why /nix/store'
  expect "the whole store" holds "in the store, which this run sees whole"
  ran "pot: explain nixcheck" sb_ explain nixcheck
  expect "says closure-store is dropped" holds "untrusted daemon relay: exposes the whole store read-only, drops closure-store"
  # shellcheck disable=SC2016 # the run's shell expands them
  ran "pot: not-found" sb_ bash -c 'ssh x; nosuchprogram; nix --version; true'
  expect "a host-only program's shim" holds "pot (work): ssh is not in this run: git fetch over ssh is the git.fetch capability"
  if ! holds "pot (work): nosuchprogram is not in this run"; then
    # shellcheck disable=SC2016 # the run's shell expands them
    echo "diagnostics: $(sb_ bash -c 'echo "BASH_ENV=$BASH_ENV"; ls -la "$BASH_ENV"; declare -F command_not_found_handle; set -o | grep -E "posix|privileged"; env | grep -c BASH_FUNC; echo "$BASHOPTS"' 2>&1 | tr '\n' ';')" >&2
  fi
  expect "bash's handler for any other" holds "pot (work): nosuchprogram is not in this run."
  expect "with the fix" holds "  permanent: tools (rebuild the root)"
  expect "and the repo kit's" holds "             or tools in the repo kit (flake.nix, fixture)"
  expect "nix without the capability" holds "pot (work): nix needs the nix capability"
  ran "pot: the handler, exported as a function too" sb_ env -u BASH_ENV bash -c 'nosuchprogram2; true'
  expect "a bash that skips BASH_ENV still has it" holds "pot (work): nosuchprogram2 is not in this run"
  refuse "pot: the scions directory is read-only" sb_ touch scions/new 2>/dev/null

  # S: git.fetch through the broker. The host's ssh is a fake that runs the remote command locally, so
  # a bound URL reaches a bare repository here
  # shellcheck disable=SC2016 # the run's shell expands them
  printf '#!/bin/sh\nfor a; do last=$a; done\necho "ssh $*" >> %s/ssh.log\nexec sh -c "$last"\n' "$work" > "$fb/ssh" && chmod +x "$fb/ssh"
  ran "pot: pot grant fetcher" tty_ $'y\n' "$work/sbr/bin/pot" grant fetcher
  # shellcheck disable=SC2016 # the run's shell expands them
  ran "pot: git.fetch" env PATH="$fb:$PATH" POT_QUIET=1 "$work/sbr/bin/pot" fetchtest -c '
    git ls-remote "ssh://git@fake.example$1" 2>&1 | grep -c refs/tags/v1.0.0
    git push "ssh://git@fake.example$1" HEAD:refs/heads/x 2>&1
    git ls-remote ssh://git@other.example/x 2>&1
    "$GIT_SSH_COMMAND" git@fake.example "ls /" 2>&1
    "$GIT_SSH_COMMAND" git@fake.example "git-upload-pack '"'"'$1'"'"'; touch /tmp/x" 2>&1; true' _ "$work/sbk.git"
  expect "a bound URL fetches" has "1"
  expect "a push is refused" holds "git-receive-pack '$work/sbk.git': only git-upload-pack (fetch) is brokered; push from the host"
  expect "an unbound URL is refused" holds "the run tried git fetch ssh://git@other.example/x (not a bound git.fetch remote)"
  expect "another command is refused" holds "the run tried ssh git@fake.example ls /: only git-upload-pack (fetch) is brokered"
  expect "extra arguments are refused" holds "touch /tmp/x: only git-upload-pack (fetch) is brokered"
  expect "the broker ran upload-pack alone" test "$(grep -c . "$work/ssh.log")" = 1
  expect "and logs each fetch" grep -q "fetch ssh://git@fake.example$work/sbk.git" "$work"/sbstate/runs/*/bindings.log

  # S: status.relay: control characters in a status text arrive cut
  # shellcheck disable=SC2016 # the run's shell expands them
  ran "pot: status.relay escapes" env PATH="$fb:$PATH" TMUX=/tmp/fake,1,0 TMUX_PANE=%9 POT_QUIET=1 "$work/sbr/bin/pot" relaytest -c \
    'workmux set-window-status "$(printf "work\033[2Jing")"; workmux send x; true'
  expect "the text arrives without its escape" grep -qxF "%9 set-window-status work[2Jing" "$work/workmux.log"
  refuse "a third command does not" grep -q "%9 send" "$work/workmux.log"
  expect "the pane is cleared after the run" grep -qxF "%9 set-window-status clear" "$work/workmux.log"

  # S: protected paths: write, mv over, rm and recreate, ln: each fails, and the file is unchanged
  cp justfile "$work/justfile.before"
  for op in 'echo x >> justfile' 'mv notes.md justfile' 'rm justfile' 'rm -f justfile; echo y > justfile' 'ln -f notes.md justfile' 'ln -sf notes.md justfile'; do
    refuse "pot: protected: $op" sb_ bash -c "$op" 2>/dev/null
  done
  expect "the protected file is unchanged" cmp -s justfile "$work/justfile.before"
  refuse "pot: a hook file is protected" sb_ bash -c 'echo x > .git/hooks/pre-commit' 2>/dev/null
  refuse "pot: git config is protected" sb_ git config core.fsmonitor "touch $work/pwned" 2>/dev/null

  # S: M3: a workspace link into a hidden path shows nothing; --read of one is refused at start
  echo hidden-secret > "$work/sbhidden/f" && ln -s "$work/sbhidden" hiddenlink
  refuse "pot: a workspace link to a hidden path is not readable" sb_ cat hiddenlink/f 2>/dev/null
  rm -f -- hiddenlink
  ln -s "$work/sbhidden" "$work/sblink"
  refused "pot: --read of a link into a hidden path" sb_ --read "$work/sblink" true
  expect "is refused at start" holds "which resolves into a hidden path"
  # S: M5: a socket in a granted --read directory is masked
  python3 -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.bind(sys.argv[1])' "$work/sbextra/s.sock"
  ran "pot: a socket in a granted directory" sb_ --read "$work/sbextra" bash -c "test -S '$work/sbextra/s.sock' && echo sock || echo masked"
  expect "is masked" has "masked"
  rm -f -- "$work/sbextra/s.sock"

  # S1 (fix round 1): git names absent at start (commondir, config.worktree, gitdir) can't be created by a run in
  # any git directory pot protects: pot puts a neutral read-only placeholder there for the run, and removes it after.
  # The qa-security repro: a run-written .git/commondir would point host git at the run's config (core.fsmonitor).
  local mk=$work/MARKER-commondir n
  # shellcheck disable=SC2016 # the run's shell expands them
  ran "pot: S1, a run tries to create .git/commondir" sb_ bash -c 'mkdir -p alt && cp -r .git/objects .git/refs .git/HEAD alt/
    printf "[core]\n\trepositoryformatversion = 0\n\tbare = false\n\tfsmonitor = \"touch %s; false\"\n" "$1" > alt/config
    echo ../alt > .git/commondir 2>/dev/null && echo "wrote commondir" || echo "commondir: refused"
    for n in config.worktree gitdir; do echo x > .git/$n 2>/dev/null && echo "wrote $n" || echo "$n: refused"; done
    echo x > .git/modules/scions/sbk/commondir 2>/dev/null && echo "wrote the scion commondir" || echo "module commondir: refused"' _ "$mk"
  expect "is refused" has "commondir: refused"
  expect "so are config.worktree" has "config.worktree: refused"
  expect "and gitdir" has "gitdir: refused"
  expect "and a scion's git directory's" has "module commondir: refused"
  for n in commondir config.worktree gitdir; do
    expect "pot: the placeholder .git/$n is gone after the run" test ! -e ".git/$n"
  done
  git_ status --short >/dev/null 2>&1 || true
  expect "host git ran none of the run's config (the marker is absent)" test ! -e "$mk"
  rm -rf -- alt

  # S1 with two runs at once in two worktrees of one repository: both plan the shared git directory's placeholders
  # (POT_TEST_FAULT=start-barrier: both resolve their pins before either prepares). Both start; the first to end
  # leaves the placeholder the other still runs with; the last one out removes it
  local wt1=$work/sbwt1 wt2=$work/sbwt2 bar=$work/sbbar p1 p2 s1=0 s2=0
  git_ worktree add -q "$wt1" -b sbwt1 && git_ worktree add -q "$wt2" -b sbwt2 && mkdir -p "$bar"
  expect "pot: before two starts at once, the repository has no commondir" test ! -e "$c/.git/commondir"
  (cd "$wt1" && POT_TEST_FAULT=start-barrier POT_TEST_BARRIER=$bar sb_ echo one) >"$work/sbwt1.out" 2>&1 &
  p1=$!
  # shellcheck disable=SC2016 # the run's shell expands them
  (cd "$wt2" && POT_TEST_FAULT=start-barrier POT_TEST_BARRIER=$bar sb_ bash -c 'for _ in $(seq 600); do
      [ -e .go ] && break; sleep 0.1; done
    echo x > "$1/commondir" 2>/dev/null && echo "wrote commondir" || echo "commondir: refused"' _ "$c/.git") >"$work/sbwt2.out" 2>&1 &
  p2=$!
  wait "$p1" || s1=$?
  out=$(cat "$work/sbwt1.out")
  expect "pot: two runs start at once in two worktrees of one repository: the first runs" test "$s1" = 0
  expect "the first ends; the placeholder stays, for the run still going" test -f "$c/.git/commondir"
  touch "$wt2/.go"
  wait "$p2" || s2=$?
  out=$(cat "$work/sbwt2.out")
  expect "the second runs too" test "$s2" = 0
  expect "and its placeholder held after the first ended" has "commondir: refused"
  expect "the last run out removes it" test ! -e "$c/.git/commondir"
  # a run killed outright (SIGKILL: its pot, its backend and its command) leaves its placeholders; the next run
  # starts, and removes them
  rm -f -- "$wt2/.go"
  (cd "$wt1" && exec setsid env -u CLAUDECODE -u CODEX_SANDBOX -u POT POT_QUIET=1 "$work/sbr/bin/pot" sleep 3301) >/dev/null 2>&1 &
  p1=$!
  for _ in $(seq 120); do pgrep -f '^sleep 3301' >/dev/null && break; sleep 0.5; done
  expect "pot: a run in a worktree has its placeholder" test -f "$c/.git/commondir"
  kill -KILL -- "-$p1" 2>/dev/null || true
  pkill -KILL -f '^sleep 3301' || true
  wait "$p1" 2>/dev/null || true
  expect "killed outright, it leaves the placeholder" test -f "$c/.git/commondir"
  # shellcheck disable=SC2016 # the inner shell expands them
  ran "pot: the next run, in the other worktree, starts" bash -c 'cd "$1" && shift && "$@"' _ "$wt2" \
    env -u CLAUDECODE -u CODEX_SANDBOX -u POT POT_QUIET=1 "$work/sbr/bin/pot" true
  expect "and removes the killed run's placeholder" test ! -e "$c/.git/commondir"
  git_ worktree remove --force "$wt1" && git_ worktree remove --force "$wt2"
  git_ branch -q -D sbwt1 sbwt2

  # --backend NAME: the named backend does the whole run, its cleanup too, never the root's default (a root with a
  # second backend: a spy that logs each verb it is given, then runs bubblewrap's)
  local spy=$work/sbspy rj
  mkdir -p "$spy"
  rj=$(grep -o '/nix/store/[^ ]*-pot-root-test\.json' "$work/sbr/bin/pot")
  # shellcheck disable=SC2016 # the spy expands them
  printf '#!/bin/sh\necho "$1" >> '"'%s/verbs'"'\nexec %s "$@"\n' "$spy" "$(jq -r .backends.bubblewrap.program "$rj")" > "$spy/backend"
  chmod +x "$spy/backend"
  jq --arg p "$spy/backend" '.backends.spy = {kind: "bwrap", program: $p}' "$rj" > "$spy/root.json"
  sed "s|$rj|$spy/root.json|" "$work/sbr/bin/pot" > "$spy/pot" && chmod +x "$spy/pot"
  ran "pot: --backend NAME, a second backend" env -u CLAUDECODE -u CODEX_SANDBOX -u POT POT_QUIET=1 "$spy/pot" --backend spy true
  expect "runs it" grep -qx run "$spy/verbs"
  expect "and it does the run's cleanup" grep -qx cleanup "$spy/verbs"

  # S: §16.11: names a run chose reach the host escaped
  # shellcheck disable=SC2016 # the run's shell expands them
  ran "pot: a run-made name with an escape byte" sb_loud bash -c 'd=$(printf "e\033]0;x\007v"); mkdir "$d" && git -C "$d" init -q'
  refuse "the exit line holds no raw escape" grep -q $'\033' <<<"$out"
  expect "and shows it escaped" holds 'e\x1b]0;x\x07v/.git'
  rm -rf -- "$c"/e*v
  ran "pot: pot why with an escape byte" sb_ why "$(printf 'x\033[2Jy')"
  refuse "prints it escaped" grep -q $'\033' <<<"$out"

  # S: a repo kit with a powerful request, granted inline at a terminal: its ordinary items together,
  # the powerful one alone, its source recorded; a changed source exits 125 with the changed paths
  # (no terminal), and prompts inline at one
  sed -i 's|kit = { protect|kit = { requests.capabilities = [ { name = "nix"; } ]; requests.network = [ "inline.example" ]; protect|' flake.nix && commit "nix"
  local objs && objs=$(git_ count-objects -v)
  ran "pot: the inline prompt, with a powerful item" tty_ $'y\ny\n' "$work/sbr/bin/pot" echo ran-after-inline
  expect "ordinary items together" holds "+ network inline.example:443"
  expect "the powerful item alone" holds "powerful: + capability nix"
  expect "and the run goes on" holds "ran-after-inline"
  ran "the grant" jq -c '[.grants, .machines[]] | map(to_entries[]) | map(select(.key | endswith("/sbc/.git") or endswith(":sbc"))) | .[0].value | [.requests, (.source.tree | length)]' "$work/sbgrants/grants.json"
  expect "holds both items and its source" has '[["capability nix","network inline.example:443"],40]'
  expect "pot writes no git object into the workspace (its source tree goes to pot's state)" test "$(git_ count-objects -v)" = "$objs"
  ran "pot: the next run proceeds" sb_ true
  ran "pot: nix, with the nix capability" sb_ nix --version
  expect "runs the host's nix" holds "nix (Nix)"
  # v2.17.0's narrowed comparison (ruling 1): a tracked change outside flake.nix, flake.lock and the kit as built
  # keeps the grant; a change to flake.nix asks again
  echo more >> notes.md && commit "a tracked change"
  ran "pot: a tracked change to another file: the grant holds" sb_ true
  echo "# a comment" >> flake.nix && commit "flake.nix changed"
  sb_ true >/dev/null 2>&1 && st=0 || st=$?
  expect "pot: a changed source exits 125" test "$st" = 125
  refused "and says why" sb_ true
  expect "with its source" holds "sbc (repo:"
  expect "changed source since its grant" holds "changed source since its grant:"
  expect "naming the changed defining file" holds "flake.nix | 1 +"
  refuse "and not the others" holds "notes.md"
  ran "pot: a changed source, at a terminal" tty_ $'y\n' "$work/sbr/bin/pot" echo ran-after-source
  expect "prompts inline" holds "changed source"
  expect "asks again for the powerful item, alone" holds "powerful: + capability nix"
  expect "with the changed paths" holds "flake.nix"
  expect "and the run goes on" holds "ran-after-source"
  sed -i 's|requests.capabilities = \[ { name = "nix"; } \]; requests.network = \[ "inline.example" \]; ||' flake.nix && commit "no nix"

  # a fixed directory's real entries (plugins' files, skills/synced) are bound into the view
  mkdir -p "$work/cfg/plugins/cache" "$work/cfg/skills/synced/s1" && echo sweep > "$work/cfg/plugins/.last_inuse_sweep"
  echo "# s1" > "$work/cfg/skills/synced/s1/SKILL.md" && ln -s "$work/sbk.git" "$work/cfg/skills/outside"
  ran "pot: pot grant fixed" tty_ $'y\n' "$work/sbr/bin/pot" grant fixed
  ran "pot: a fixed directory with real entries" sb_ fixedtest -c 'cat ~/.claude/plugins/.last_inuse_sweep; cat ~/.claude/skills/synced/s1/SKILL.md; ls -A ~/.claude/skills; touch ~/.claude/plugins/x 2>&1; true'
  expect "a real file" holds "sweep"
  expect "a real folder skillDirs names" holds "# s1"
  refuse "a link outside the read requests is left out" holds "outside"
  expect "read-only" holds "Read-only file system"
  ran "pot: what is left out" sb_loud fixedtest -c true
  expect "is one short note" holds "fixed: 1 entry left out of ~/.claude/skills (pot explain)"
  ran "pot: explain lists it" sb_ explain fixedtest
  expect "by name" holds "left out: outside"
  # host memory: read-only, file by file; what the run writes is copied out for review
  local key && key=$(pwd -P | sed 's/[^A-Za-z0-9]/-/g')
  mkdir -p "$work/cfg/projects/$key/memory" && echo host-memory > "$work/cfg/projects/$key/memory/m.md"
  echo "- [m](m.md)" > "$work/cfg/projects/$key/memory/MEMORY.md"
  ran "pot: host memory" sb_loud fixedtest -c "cat ~/.claude/projects/$key/memory/m.md; echo x >> ~/.claude/projects/$key/memory/m.md 2>&1; echo new > ~/.claude/projects/$key/memory/new.md; echo '- [new](new.md)' >> ~/.claude/projects/$key/memory/MEMORY.md"
  expect "is read" holds "host-memory"
  expect "read-only" holds "Read-only file system"
  expect "the exit line names only what the run wrote" holdsj "pot: the run wrote 2 memory files (MEMORY.md, new.md): copies in "
  expect "MEMORY.md, a writable copy, is copied out changed" grep -q "new.md" "$(find "$work/sbstate/runs" -path '*/memory-out/*/MEMORY.md' | head -1)"
  expect "and never written back" test "$(cat "$work/cfg/projects/$key/memory/MEMORY.md")" = "- [m](m.md)"
  expect "copied out" test -n "$(ls "$work"/sbstate/runs/*/memory-out/*/new.md)"
  refuse "and never the placeholders" test -n "$(ls "$work"/sbstate/runs/*/memory-out/*/m.md 2>/dev/null)"
  expect "the host's memory is unchanged" test "$(cat "$work/cfg/projects/$key/memory/m.md")" = host-memory

  # the claude-code kit's overlay: a failed call's denial reaches the model through PostToolUseFailure
  ran "pot: pot grant claude-code" tty_ $'y\ny\n' "$work/sbr/bin/pot" grant claude-code
  ran "pot: the overlay's hooks" sb_ claude
  expect "SessionStart says how memory works here" holds "MEMORY.md is a writable copy"
  expect "a failed call's denial reaches the model" holds '"hookEventName": "PostToolUseFailure"'
  expect "with its text" holds "the run tried denied-tool.example:443 (not in this run)"

  # one name for a kit everywhere; KIT names every instance
  ran "pot: pot grant pair" tty_ $'y\ny\n' "$work/sbr/bin/pot" grant pair
  expect "lists both instances" holds "pair@a"
  expect "and both" holds "pair@b"
  ran "pot: --revoke pair, at a terminal" tty_ $'y\n' "$work/sbr/bin/pot" grant --revoke pair
  expect "confirms the list" holds "pair names 2 grants:"
  expect "and revokes both" test "$(jq '[.grants | keys[] | select(startswith("root:pair"))] | length' "$work/sbgrants/grants.json")" = 0
  git_ remote add origin https://example.com/o/sbname.git
  ran "pot: a repo kit's name comes from its origin" sb_loud true
  expect "on the startup line" holds "pot: work · sbname ·"
  ran "pot: pot grant --diff" sb_ grant --diff
  expect "uses it too (the identity its name implies is not repeated)" grep -qE "^ok +sbname +requests nothing$" <<<"$out"
  git_ remote remove origin
  refused "pot: pot grant --root" sb_ grant --root x --workspace .
  expect "comes later" holds "pot grant --root comes with deployment roots, in a later release"

  # explain and why
  ran "pot: explain" sb_ explain
  expect "the context" holds "context    work: in-place, access write, bubblewrap; includes root:daniel"
  expect "the repo kit's protect" grep -qE 'AGENTS\.md .*\(repo\)' <<<"$out"
  expect "the scion kit's" holds "sbk.yaml (sbk)"
  refuse "no truncated kinds" holds "capabili "
  ran "pot: explain --json" sb_ explain --json
  expect "is policy.json v1 (its children table: sbk-tool)" test "$(jq -c '[.version, .context, .mode, (.children|length)]' <<<"$out")" = '[1,"work","in-place",1]'
  expect "and validates against policy.json/1" valid contracts/policy-1.schema.json
  ran "pot: explain --backend-command" sb_ explain --backend-command true
  expect "prints bwrap's" holds "--disable-userns"
  ran "pot: why a credential's variable" sb_ why TESTTOK
  expect "names its kit and binding" has "TESTTOK is carried by relay's credential (file $work/sbtok, as a file in /run/pot/secrets that its variable names (the run holds it)), in pot relaytest runs"
  ran "pot: explain --backend-command host" sb_ explain --backend-command host echo hi
  expect "prints its argv" holds "argv       echo hi"
  ran "pot: a port-80 denial" sb_ bash -c "$ask; cat /run/pot/denials" _ denied.example:80
  expect "suggests HOST:80" holds "one run:   pot --net denied.example:80 …"
  ran "pot: why HOST" sb_ why other.example
  expect "says would" has "pot (work): pot would refuse other.example:443 in a run here."
  expect "one run" has "  one run:   pot --net other.example …"
  expect "permanent" has "  permanent: requests.network in the repo kit (flake.nix, fixture)"
  expect "and names the included kit" has "             or kits.daniel.requests.network, for every repository (rebuild the root)"
  ran "pot: why, on the host" sb_ why justfile
  expect "protected" has "pot (work): justfile is protected (**/justfile, pot): edit it on the host"
  ran "pot: why, inside a run" sb_ bash -c 'pot why denied.example; pot why nothing.invalid:1; pot why ~/.ssh'
  expect "a host not in the run" holds "pot (work): the run tried denied.example:443 (not in this run)"
  expect "a granted host" holds "nothing.invalid:1 is granted (daniel); in this run"
  # shellcheck disable=SC2088 # pot prints ~ literally
  expect "a hidden path" holds "~/.ssh is hidden by your root in every run"
  refused "pot: pot inside a run" sb_ pot true
  expect "starts no run (v2.20.0: a kit command's wrapper starts a sibling)" holds "pot (work): pot inside a run starts no run: a kit command's wrapper starts it beside this run; run anything else on the host"
  refused "pot: pot grant inside a run" sb_ pot grant
  expect "is the host's" holds "pot grant is for people, on the host"

  # the bundle's protect, and the flake directory pot evaluated: labelled by who asks, never graft
  ran "pot: explain shows the bundle's protect" sb_ explain
  expect "every kit-owner.nix and the scions directory, as the repo's" grep -qE '\*\*/kit-owner\.nix scions .*\(repo\)' <<<"$out"
  refuse "and no graft.nix of pot's own" holds "**/graft.nix"
  refuse "and no (graft) label" holds "(graft)"
  refuse "pot: the bundle's scions directory is read-only" sb_ touch scions/new2 2>/dev/null
  # pot why --json (contract run-env/1): exit 0 when reached, 1 when not
  ran "pot: why --json, on the host, a granted host" sb_ why nothing.invalid:1 --json
  expect "reached" test "$(jq -c '[.version, .kind, .reach, .reached]' <<<"$out")" = '[1,"network","nothing.invalid:1",true]'
  refused "pot: why --json, a host not reached" sb_ why https://other.example/o/r.git --json
  expect "says not reached, exit 1" test "$(jq -c '[.kind, .reach, .reached]' <<<"$out")" = '["network","other.example:443",false]'
  refused "pot: why --json, an ssh URL without a git.fetch binding in this run" sb_ why ssh://git@fake.example/x.git --json
  expect "is git.fetch, not reached" test "$(jq -c '[.kind, .reached]' <<<"$out")" = '["git.fetch",false]'
  refused "pot: why --json, something else" sb_ why bash --json
  expect "is usage" holds "pot why --json answers a host, HOST:PORT, a git URL or a path"
  # shellcheck disable=SC2016 # the run's bash expands them
  ran "pot: why --json, inside a run" sb_ bash -c 'pot why nothing.invalid:1 --json; echo "exit $?"; pot why https://other.example/x --json; echo "exit $?"'
  expect "reached, exit 0" has "exit 0"
  expect "not reached, exit 1" has "exit 1"
  expect "the JSON" holds '"reach": "other.example:443"'

  # discovery from git facts (contract pot-kits/1): pot evaluates only a flake located from .gitmodules
  # at HEAD; pot-kits, else v2.15.0's graft-kits; neither is no repo kit, cached by tree
  local d
  d=$work/sbnosub && mkdir -p "$d" && git_ -C "$d" init -q
  printf 'this is not a flake {\n' > "$d/flake.nix"
  (cd "$d" && commit init && ran "pot: a repository without submodules: its flake is never evaluated" sb_loud true \
    && refuse "no build" holds "building the repo kit" \
    && refuse "and no warning of a missing pot-kits" holds "has no pot-kits")
  d=$work/sbold15 && mkdir -p "$d" && git_ -C "$d" init -q
  cat > "$d/flake.nix" <<EOF
{
  inputs.pot.url = "path:$pot";
  outputs = { pot, ... }: let pkgs = pot.inputs.nixpkgs.legacyPackages.$sys; in {
    # v2.15.0's bundle, as its mkFlake wrote it
    packages.$sys.graft-kits = pkgs.runCommand "graft-kits" { } ''
      mkdir -p \$out/bin
      echo '{"version":1,"dir":"scions","file":"flake.nix (perSystem)","self":{"file":"flake.nix (perSystem)","protect":["OLD.md"]},"scions":{}}' > \$out/kit.json
      touch \$out/closure
    '';
  };
}
EOF
  (cd "$d" && echo old > OLD.md && by_hand sbk && commit init && quiet nix flake lock && commit lock \
    && refused "pot: a v2.15.0 consumer (graft-kits only)" sb_ true \
    && expect "is refused, naming pot-kits" holds "pot (work): $d has graft-kits only: its graft predates v2.16.0, and this pot reads pot-kits: take graft v2.16.0 or later there (nix flake update graft), or run pot host CMD")
  # a bundle graft v2.16.x built may hold packages: pot reads it as tools, with no warning (ruling 2)
  d=$work/sbpk16 && mkdir -p "$d" && git_ -C "$d" init -q
  cat > "$d/flake.nix" <<EOF
{
  inputs.pot.url = "path:$pot";
  outputs = { pot, ... }: let pkgs = pot.inputs.nixpkgs.legacyPackages.$sys; in {
    packages.$sys.pot-kits = pkgs.runCommand "pot-kits" { } ''
      mkdir -p \$out
      echo '{"version":1,"self":{"file":"flake.nix (perSystem)","packages":["\${pkgs.hello}"]},"scions":{},"protect":[]}' > \$out/kit.json
      ln -s \${pkgs.hello}/bin \$out/bin
      cat \${pkgs.closureInfo { rootPaths = [ pkgs.hello ]; }}/store-paths > \$out/closure
    '';
  };
}
EOF
  (cd "$d" && by_hand sbk && commit init && quiet nix flake lock && commit lock \
    && ran "pot: a v2.16.x bundle whose kit has packages" sb_loud hello \
    && expect "runs, its package on PATH" holds "Hello, world!" \
    && refuse "with no warning" holds "old name")
  d=$work/sbnokit && mkdir -p "$d" && git_ -C "$d" init -q
  printf '{\n  inputs.pot.url = "path:%s";\n  outputs = { pot, ... }: { packages.%s.other = pot.packages.%s.default; };\n}\n' "$pot" "$sys" "$sys" > "$d/flake.nix"
  (cd "$d" && by_hand sbk && commit init && quiet nix flake lock && commit lock \
    && ran "pot: a flake with no pot-kits" sb_loud true \
    && refuse "has no repo kit (no build)" holds "building the repo kit" \
    && expect "and the answer is cached by tree" grep -lq '"out": null' "$work"/sbstate/cache/kits/*.json \
    && expect "pot says its kits and protections are not applied (0.1.0, decision 6)" \
      has "pot (work): $d's flake has no pot-kits; its own kits and protections are not applied" \
    && expect "once" test "$(grep -c "has no pot-kits" <<<"$out")" = 1 \
    && ran "pot: again, the answer cached" sb_ true \
    && expect "says it again, under POT_QUIET too" has "pot (work): $d's flake has no pot-kits; its own kits and protections are not applied" \
    && ran "pot: explain there" sb_ explain \
    && expect "says it too" holds "$d's flake has no pot-kits")
  d=$work/sbv2 && mkdir -p "$d" && git_ -C "$d" init -q
  cat > "$d/flake.nix" <<EOF
{
  inputs.pot.url = "path:$pot";
  outputs = { pot, ... }: let pkgs = pot.inputs.nixpkgs.legacyPackages.$sys; in {
    packages.$sys.pot-kits = pkgs.runCommand "pot-kits" { } ''
      mkdir -p \$out/bin && echo '{"version":2,"self":null,"scions":{},"protect":[]}' > \$out/kit.json && touch \$out/closure
    '';
  };
}
EOF
  (cd "$d" && by_hand sbk && commit init && quiet nix flake lock && commit lock \
    && refused "pot: a newer bundle" sb_ true \
    && expect "is refused, naming its version" holds "kit.json is version 2, and this pot reads version 1: take a pot that reads it, or run pot host CMD")
  # .gitmodules names a path with no gitlink at HEAD: no fact, so its flake is never evaluated (C2)
  d=$work/sbstale && mkdir -p "$d/nix" && git_ -C "$d" init -q
  printf '[submodule "nix/gone"]\n\tpath = nix/gone\n\turl = file:///nowhere\n' > "$d/.gitmodules"
  printf 'throw "this flake must not be evaluated"\n' > "$d/nix/flake.nix"
  # a lock, so that only the gitlink rule (not a missing flake.lock) keeps pot from evaluating it (R2-1)
  printf '{"nodes":{"root":{}},"root":"root","version":7}\n' > "$d/nix/flake.lock"
  (cd "$d" && commit init && ran "pot: a stale .gitmodules entry" sb_loud true \
    && refuse "evaluates nothing" holds "must not be evaluated" && refuse "builds nothing" holds "building the repo kit")
  cd "$c"
  # pot grant takes the name pot displays: this repository's (its folder, a local origin) (R4)
  sb_ grant sbc </dev/null >/dev/null 2>"$work/r4.err" || true
  refuse "pot: pot grant NAME, the displayed name" grep -q "no kit 'sbc' here" "$work/r4.err"
  # pot host CMD finds a command of the workspace's bundle that the host's PATH lacks (F7)
  ran "pot: pot host, a bundle kit's command" sb_ host sbk-tool
  expect "runs it" holds "sbk-tool hi"
  # an unverified scion (its gitlink not a release tag): one start note, no 125; its command runs without its
  # kit's reach (architecture §4, v2.17.0)
  git_ -C "$work/src/sbk" commit -q --allow-empty -m "untagged" && git_ -C "$work/src/sbk" push -q origin HEAD:refs/heads/main
  git_ -C scions/sbk fetch -q origin && git_ -C scions/sbk checkout -q --detach origin/main && git_ add scions/sbk && commit "sbk untagged"
  ran "pot: an unverified scion" sb_loud true
  expect "one start note" holds "sbk: gitlink is not a release tag at its URL; its kit is unverified and its grant does not apply"
  ran "pot: its command runs, with no 125" sb_ sbk-tool
  expect "without its reach" holds "sbk-tool hi"
  git_ -C scions/sbk checkout -q --detach v1.0.0 && git_ add scions/sbk && commit "sbk v1.0.0 again"
  # a protected folder is read-only throughout: a file inside a scion, not only its graft.nix (B)
  refuse "pot: a scion's file is read-only" sb_ bash -c 'echo x >> scions/sbk/README.md' 2>/dev/null
  # (runs and doctor are v2.17.0's: sec_dispatch); an unknown command says where a program for every run goes (C9)
  refused "pot: an unknown command" sb_ no-such-program-x
  local rc=0 && { sb_ no-such-program-x >/dev/null 2>&1 || rc=$?; }
  expect "exits 2" test "$rc" = 2
  expect "says the host lacks it too (v2.16.1)" holds "pot (work): no-such-program-x is not in this run, nor on the host's PATH."
  expect "how to get it" holds "  install it (for a repository's own tools, its package manager), then:"
  expect "points at the root's tools" holds "  permanent: tools (rebuild the root)"
  # the same through a context and after --
  for a in "work no-such-program-x" "work -- no-such-program-x" "-- no-such-program-x"; do
    # shellcheck disable=SC2086 # the words are the test
    refused "pot: $a" sb_ $a
    expect "the same message" holds "pot (work): no-such-program-x is not in this run, nor on the host's PATH."
  done
  # a missing path with / is not reached (--json), as its text says
  refused "pot: why --json, a missing path" sb_ why nope/x.nix --json
  expect "reached false" test "$(jq -c '[.kind, .reached]' <<<"$out")" = '["file",false]'
  expect "text: no such path" holds "nope/x.nix: no such path here"
  # pot why --json on the host answers paths from the default context's policy (C4), and git URLs as git (F10)
  refused "pot: why --json, a hidden path" sb_ why "$work/sbhidden/x" --json
  expect "not reached, from the policy" test "$(jq -c '[.kind, .reached]' <<<"$out")" = '["file",false]'
  ran "pot: why --json, a workspace file" sb_ why notes.md --json
  expect "reached" test "$(jq -c '[.kind, .reached]' <<<"$out")" = '["file",true]'
  ran "pot: why, an ssh URL as text" sb_ why ssh://git@fake.example/x.git
  expect "is about git.fetch, not a path" holds "is not a bound git.fetch remote"

  # grants: grown sets, arrival by git, a file that does not parse
  sed -i 's|kit = { protect|kit = { requests.network = [ "more.example" ]; protect|' flake.nix && commit "more"
  refused "pot: a repo kit that asks for more" sb_ true
  expect "stops" holds "asks for more than its grant:"
  expect "with the item" holds "  + network more.example:443"
  ran "pot: pot grant --diff" sb_ grant --diff
  expect "names it" holds "  + network more.example:443"
  expect "after a summary line" grep -qE '^[0-9]+ kits? (granted|asks for more)' <<<"$out"
  expect "and its kit's line, with what to run (ASCII: not a terminal)" grep -qE '^~ +sbc +asks for more \(1 more\) · pot grant sbc$' <<<"$out"
  # a word that reads as a file name, but names no path, is not a host (v2.16.1)
  ran "pot: why a file-like word" sb_ why no-such-file.nix
  expect "says no such path" holds "no-such-file.nix: no such path here (pot why PATH answers paths; for a host, pot why no-such-file.nix:443)"
  refuse "never offers --net" holds "--net"
  sed -i 's|requests.network = \[ "more.example" \]; ||' flake.nix && commit "less"
  cp "$work/sbgrants/grants.json" "$work/sbgrants.bak"
  jq '.grants["root:other"] = {"digest": "sha256-c8s4WKaHqElMozIwUwFigvPa051Cz2LKTnndoqrH2aw=", "requests": ["x"]}' "$work/sbgrants.bak" > "$work/sbgrants/grants.json"
  ran "pot: a grants file changed elsewhere" sb_loud true
  expect "one line" holds "pot: the grants file changed since this machine last read it: pot grant --diff"
  refuse "never says git" holds "arrived by git"
  jq '.grants["root:daniel"].digest = "sha256-AAAA"' "$work/sbgrants.bak" > "$work/sbgrants/grants.json"
  refused "pot: a digest that does not match" sb_ true
  expect "names the repair" holds "pot grant --revoke daniel, then pot grant daniel"
  ran "pot: pot grant --revoke repairs it" sb_ grant --revoke daniel
  ran "pot: and the grants file parses again" jq -e '.grants | has("root:daniel") | not' "$work/sbgrants/grants.json"
  echo '{' > "$work/sbgrants/grants.json"
  refused "pot: a grants file that does not parse" sb_ true
  expect "stops every run" holds "the grants file does not parse"
  cp "$work/sbgrants.bak" "$work/sbgrants/grants.json"

  # root repositories, contexts, verbs (X9: its commits come back for review)
  (cd "$work/sbrootrepo" && refused "pot: work in a root repository" sb_ true \
    && expect "is refused, naming dispatch (D13, X9)" holds "is a root: pot dispatch CMD (its commits come back for review), or pot inspect CMD; pot host CMD runs it unsandboxed")
  refused "pot: --repo to a root repository" sb_ --repo "$work/sbrootrepo" true
  expect "is refused, naming the flag (V1)" holds "--repo $work/sbrootrepo is the root $work/sbrootrepo: a run never writes a root (pot inspect reads it; pot host CMD runs unsandboxed)"
  # S3 and V1: a write item that is, sits inside or holds a root repository is refused at start, naming it
  mkdir -p "$work/sbrootrepo/sub"
  refused "pot: S3, --write of the root" sb_ --write "$work/sbrootrepo" true
  expect "names the flag and the root (V1)" holds "--write $work/sbrootrepo is the root $work/sbrootrepo: a run never writes a root (pot inspect reads it; pot host CMD runs unsandboxed)"
  refused "pot: S3, --write inside the root" sb_ --write "$work/sbrootrepo/sub" true
  expect "says it is inside the root (V1)" holds "--write $work/sbrootrepo/sub is inside the root $work/sbrootrepo: a run never writes a root (pot inspect reads it; pot host CMD runs unsandboxed)"
  # a root by its origin, nested in another repository: --write and --repo of the outer one are refused too
  local ou=$work/sbouter && mkdir -p "$ou/inner" && git_ -C "$ou" init -q && git_ -C "$ou" commit -q --allow-empty -m o
  git_ -C "$ou/inner" init -q && git_ -C "$ou/inner" remote add origin ssh://git@fake.example/rootx.git && git_ -C "$ou/inner" commit -q --allow-empty -m i
  refused "pot: S3, --write of a folder holding a root (by origin)" sb_ --write "$ou" true
  expect "names the root (V1)" holds "--write $ou holds the root $ou/inner: a run never writes a root (pot inspect reads it; pot host CMD runs unsandboxed)"
  refused "pot: S3, --repo of a repository holding a root" sb_ --repo "$ou" true
  expect "names the root (V1)" holds "--repo $ou holds the root $ou/inner: a run never writes a root (pot inspect reads it; pot host CMD runs unsandboxed)"
  ran "pot: S3, --repo of it with --access read" sb_ --access read --repo "$ou" true
  # D5: in a dispatch, --repo of a root gets the root refusal, before --repo's later-release one
  refused "pot: D5, dispatch --repo of a root" sb_ dispatch --repo "$work/sbrootrepo" true
  expect "D5: the root refusal (V1)" holds "pot (dispatch): --repo $work/sbrootrepo is the root $work/sbrootrepo: a run never writes a root (pot inspect reads it; pot host CMD runs unsandboxed)"
  refused "pot: D5, dispatch --repo of a repository holding a root" sb_ dispatch --repo "$ou" true
  expect "D5: names the root it holds" holds "pot (dispatch): --repo $ou holds the root $ou/inner: a run never writes a root"
  # a granted write request naming a root
  ran "pot: S3, grant a kit's write of a root" tty_ $'y\n' "$work/sbr/bin/pot" grant rootwriter
  refused "pot: S3, its command" sb_ rwtest -c true
  expect "is refused at start, naming the kit (V1)" holds "rootwriter's write $work/sbrootrepo is the root $work/sbrootrepo: a run never writes a root (pot inspect reads it; pot host CMD runs unsandboxed)"
  refused "pot: S3, --write of a root from a dispatch (it would write the root in place)" sb_ dispatch --write "$work/sbrootrepo" true
  expect "is refused too (V1)" holds "pot (dispatch): --write $work/sbrootrepo is the root $work/sbrootrepo: a run never writes a root (pot inspect reads it; pot host CMD runs unsandboxed)"
  refused "pot: run with no --command (the wrappers' entry)" sb_ run true
  expect "says its form" holds "pot run --command NAME --manifest PATH -- ARGS (the wrappers' entry)"
  refused "pot: serve is reserved" sb_ serve
  refused "pot: outside a git repository" bash -c "cd '$work' && POT_QUIET=1 '$work/sbr/bin/pot' true"
  expect "names inspect and host" holds "is not inside a git repository: pot inspect CMD, or pot host CMD"
  ran "pot: host" sb_loud host true
  expect "says no sandbox" holds "pot: host · no sandbox · logged as "
  refused "pot: an unknown first word" sb_ nosuch
  expect "may be a context" holds "(nosuch is not a context either: "
  refused "pot: a host program not in the run" sb_ nix --version
  expect "says how to add it" holds "pot (work): nix is not in this run."
  expect "one run" holds "  one run:   pot --package PKG …"

  # inspect (D20): its own pid namespace, the workspace read-only, secrets and sockets masked. In the
  # root repository (no flake: a path: flake directory can't hold a socket)
  (
    cd "$work/sbrootrepo"
    echo "TOKEN=secret-value" > .env && echo "keep" > .envrc.example
    python3 -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.bind(sys.argv[1])' "$work/sbrootrepo/pot.sock"
    # shellcheck disable=SC2016 # the run's shell expands them
    ran "pot: inspect, in a root repository" sb_ inspect bash -c 'ps -e | wc -l; echo "[$(cat .env)]"; cat .envrc.example; test -S pot.sock && echo sock || echo nosock; touch x 2>&1; true'
    expect "its own processes" test "$(head -1 <<<"$out")" -lt 6
    expect "a secret is empty" holds "[]"
    refuse "its value never shows" holds "secret-value"
    expect "other files read" holds "keep"
    expect "a socket masked" holds "nosock"
    expect "nothing written" holds "Read-only file system"
    rm -f -- .env .envrc.example pot.sock
  )

  # pot init
  local i=$work/sbinit
  ran "pot: init" env POT_INIT_URL="path:$pot" "$work/sbr/bin/pot" init "$i"
  expect "writes a flake" test -f "$i/flake.nix"
  expect "names mkRoot" grep -q "pot.lib.mkRoot" "$i/flake.nix"
  expect "with claude-code, its token file and nix" grep -q 'claude-token-personal' "$i/flake.nix"
  expect "and prints how to put pot on PATH" holds "nix profile install path:$i#pot"
  refuse "never an ln" holds "ln -s"
  refused "pot: init again" "$work/sbr/bin/pot" init "$i"
  refused "pot: init with no folder" "$work/sbr/bin/pot" init
  expect "asks for one" holds "pot init DIR: name the folder"
  expect "takes pot from POT_INIT_URL" grep -qF "inputs.pot.url = \"path:$pot\";" "$i/flake.nix"
  expect "and its nixpkgs" grep -qF 'inputs.nixpkgs.follows = "pot/nixpkgs";' "$i/flake.nix"
  expect "the claude-code kit is pot's" grep -qF 'pot.kits.claude-code { inherit pkgs; binary = "host"; }' "$i/flake.nix"
  refuse "it never names graft" grep -qi graft "$i/flake.nix"
  local i2=$work/sbinit2
  ran "pot: init with no POT_INIT_URL" env -u POT_INIT_URL "$work/sbr/bin/pot" init "$i2"
  local release; release=$(sed -n 's/^VERSION = "\(.*\)"$/\1/p' "$pot/core/pot.py")
  expect "pins this pot's release" grep -qF "inputs.pot.url = \"github:danielnaab/pot/v$release\";" "$i2/flake.nix"
  (cd "$i" && quiet nix flake lock)
  ran "pot: the init flake builds" nix build --no-link "path:$i#packages.$sys.pot"
  expect "pot: runs/1, every log line and record this section's runs wrote validates" runs_valid "$work/sbstate" sb_
  expect "pot: policy.json/1, every policy.json this section's runs wrote validates" policies_valid "$work/sbstate"
  expect "pot: launch.json/1, every launch.json this section's runs wrote validates" launches_valid "$work/sbstate"
  # 0.3.0: pot's core resolves an in-place write run's pins once and passes them (bwrap binds what they list)
  # shellcheck disable=SC2016 # $1 is the inner shell's
  expect "pot: 0.3.0, a run with write access in place has its workspace's pins in launch.json" quiet \
    sh -c 'find "$1" -name launch.json -size +0 -exec cat {} + | jq -se "any(.[]; .pins[0].ro | length > 0)"' _ "$work/sbstate"
  cd "$work"
}

# dispatch (pot/, v2.17.0): clone mode, fetch-back through staging, the run log as runs/1 (pot runs,
# pot review), a 125's stopped record and the pane, and the resource scope. Its own test root and workspace.
# pot_seatbelt CONSUMER: the pot section on macOS, the Seatbelt backend (core/seatbelt.py), with sec_pot's test
# root ($work/sbr) and consumer. What Seatbelt claims (closure-store no-tiocsti in-place private-proxy), what it
# refuses (nesting, listening), and what stands in for mounts (a HOME of links, path rules, the run's folder).
pot_seatbelt() {
  local c=$1 st
  cd "$c"
  # shellcheck disable=SC2317 # run through ran, expect and refuse
  sb_loud() { env -u CLAUDECODE -u CODEX_SANDBOX -u POT "$work/sbr/bin/pot" "$@"; }
  # shellcheck disable=SC2317
  sb_() { POT_QUIET=1 sb_loud "$@"; }
  # tty_ ANSWERS CMD: CMD on a terminal (BSD script), its prompts answered from ANSWERS
  # shellcheck disable=SC2317
  tty_() { local a=$1; shift; (sleep 3; printf '%s' "$a"; sleep 3) | script -q /dev/null env -u CLAUDECODE -u CODEX_SANDBOX -u POT "$@" | tr -d '\r'; }

  # (it exits 1: the fixture's binding files are missing, each a ! line)
  out=$(sb_ doctor 2>&1) || true
  expect "names the backend and what it proves" holds "backend    seatbelt: sandbox-exec works; proves closure-store no-tiocsti in-place private-proxy; lacks own-netns own-pidns"
  expect "and no resource limits" holds "resources  Seatbelt has no resource limits"
  refused "pot (seatbelt): never granted, no terminal" sb_loud true
  expect "stops with the diff" holds "pot (work): daniel (root:daniel) requests reach that is not granted:"
  sb_ true >/dev/null 2>&1 && st=0 || st=$?
  expect "exit 125" test "$st" = 125
  ran "pot (seatbelt): the inline first grant, at a terminal" tty_ $'y\n' "$work/sbr/bin/pot" echo ran-after-grant
  expect "and the run continues" holds "ran-after-grant"

  ran "pot (seatbelt): a command runs, in the repository" sb_loud pwd
  expect "there" has "$(pwd -P)"
  expect "the startup line names what Seatbelt lacks" holds "seatbelt lacks own-netns own-pidns"
  # shellcheck disable=SC2016 # the run's shell expands them
  ran "pot (seatbelt): the run's environment" sb_ bash -c 'echo "ctx=$POT home=$HOME tmp=$TMPDIR"; ls -A ~; hello; jq -r .context "$POT_POLICY"; head -1 "${POT_POLICY%/*}/README.md"'
  expect "HOME is the run's own" holds "home=$(cd "$work" && pwd -P)/sbstate/runs/"
  expect "with the hidden list and the gitconfig" holds ".pot-hidden"
  expect "the repo kit's package" holds "Hello, world!"
  expect "POT_POLICY is the run's policy.json" has "work"
  expect "the README beside it" has "# pot: this run (work)"
  ran "pot (seatbelt): the caller's variables" env SB_DECOY=decoy-secret POT_QUIET=1 "$work/sbr/bin/pot" bash -c 'env | grep -c decoy-secret || true'
  expect "none of them" has "0"
  # 0.2.0: access none, no workspace: denied, and the run starts in its own HOME
  # shellcheck disable=SC2016 # the run's shell expands them
  ran "pot (seatbelt): access none" sb_ blind bash -c 'echo "pwd=$PWD home=$HOME"; cat "$1/notes.md" >/dev/null 2>&1 && echo "workspace seen"
    cat "$1/.git/HEAD" >/dev/null 2>&1 && echo "git seen"; true' _ "$c"
  expect "starts in its HOME" grep -qE '^pwd=(.*) home=\1$' <<<"$out"
  refuse "the workspace is denied" holds "workspace seen"
  refuse "and its git directory" holds "git seen"
  # shellcheck disable=SC2016 # the run's shell expands them
  ran "pot (seatbelt): access none, inspect's view" sb_ blindview bash -c 'cat "$1/notes.md" >/dev/null 2>&1 && echo "workspace seen"; true' _ "$c"
  refuse "denies the workspace too" holds "workspace seen"
  ran "pot (seatbelt): explain --backend-command" sb_ explain --backend-command
  expect "is sandbox-exec, with the profile" holds "/usr/bin/sandbox-exec -f"
  expect "and every path a parameter" holds "-DP0="

  # protections: path rules, not mounts
  for p in AGENTS.md sbk.yaml flake.nix .gitmodules .git/config; do
    refuse "pot (seatbelt): read-only: $p" sb_ bash -c "echo x >> $p" 2>/dev/null
  done
  refuse "pot (seatbelt): no new hard link to a protected file" sb_ ln AGENTS.md agents-link 2>/dev/null
  expect "pot (seatbelt): the repository is writable" sb_ bash -c 'echo y >> notes.md'
  # v2.19.2: on APFS another spelling (case, a Kelvin sign) is the same file, and refused as it is
  for p in .GIT/CONFIG FLAKE.NIX "$(printf 'fla\xe2\x84\xaae.nix')"; do
    refuse "pot (seatbelt): read-only, another spelling: $p" sb_ bash -c "echo x >> '$p'" 2>/dev/null
  done
  # a host-run file created in another spelling is what host direnv opens: reported, and protected in the next run
  ran "pot (seatbelt): a created .Envrc" sb_loud bash -c 'echo "echo planted" > .Envrc'
  expect "is reported after the run" holdsj "pot: WARNING: the run created what host git, a shell prompt, an editor or direnv reads, and may run code from: .Envrc"
  refuse "and is read-only in the next run, in any spelling" sb_ bash -c 'echo x >> .ENVRC' 2>/dev/null
  rm -f -- .Envrc
  refuse "pot (seatbelt): .git can't be renamed" sb_ mv .git .git2 2>/dev/null
  refuse "nor a scion's folder" sb_ mv scions/sbk scions/sbk2 2>/dev/null
  refuse "nor the run's HOME link to its hidden list" sb_ bash -c 'rm ~/.pot-hidden' 2>/dev/null
  ran "pot (seatbelt): a commit inside" sb_ bash -c 'git add notes.md && git commit -qm inside && git log -1 --format=%s'
  expect "is made" has "inside"
  ran "pot (seatbelt): a redirected stdout (a regular file)" sb_ cat notes.md
  sb_ cat AGENTS.md > "$work/sb-out.txt"
  expect "takes what the run writes" grep -q "# agents" "$work/sb-out.txt"
  ln AGENTS.md "$work/pot-agents-link"
  refused "pot (seatbelt): a protected file with another hard link" sb_ true
  expect "stops the run" holds "pot (work): AGENTS.md has another hard link"
  rm -f -- "$work/pot-agents-link"

  # what the run can't read: the hidden list, pot's state, the grants file, the real home, other store paths
  mkdir -p "$work/sbhidden" && echo s > "$work/sbhidden/x"
  refuse "pot (seatbelt): a hidden path" sb_ cat "$work/sbhidden/x" 2>/dev/null
  refuse "pot (seatbelt): pot's state" sb_ ls "$work/sbstate" 2>/dev/null
  refuse "pot (seatbelt): the grants file" sb_ cat "$work/sbgrants/grants.json" 2>/dev/null
  refuse "pot (seatbelt): the real home" sb_ ls "$(eval echo "~$(id -un)")" 2>/dev/null
  refuse "pot (seatbelt): a store path outside the closure" sb_ ls "$(nix build --no-link --print-out-paths "$np#cowsay")" 2>/dev/null
  rm -rf -- "$work/sbhidden/x"

  # the network: the proxy's port and the run's token, nothing else
  # shellcheck disable=SC2016
  ran "pot (seatbelt): a host not granted" sb_ bash -c 'curl -sS -m 5 https://denied.example/ 2>&1; cat "${POT_POLICY%/*}/denials"'
  expect "is refused by the proxy" holds "CONNECT tunnel failed, response 403"
  expect "and the run's denials say so" holds "pot (work): the run tried denied.example:443 (not in this run)."
  # shellcheck disable=SC2016
  ran "pot (seatbelt): the proxy without the run's token" sb_ bash -c 'p=${HTTPS_PROXY##*@}; curl -sS -m 5 -x "http://$p" https://denied.example/ 2>&1 || true'
  expect "is refused (407)" holds "407"
  # a port that listens on the host: the run can't reach it (no proxy, no rule)
  python3 -c 'import socket, sys, time; s = socket.socket(); s.bind(("127.0.0.1", 0)); s.listen(); print(s.getsockname()[1], flush=True); time.sleep(60)' > "$work/sb-port" &
  local lp=$! && sleep 1
  refused "pot (seatbelt): a host port, around the proxy" sb_ python3 -I -c "import socket; socket.create_connection(('127.0.0.1', $(cat "$work/sb-port")), 3)"
  expect "is refused" holds "PermissionError: [Errno 1] Operation not permitted"
  kill "$lp" 2>/dev/null || true
  refused "pot (seatbelt): listening on a port" sb_ python3 -I -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); s.listen()'
  expect "is refused (local-listen)" holds "Operation not permitted"
  ran "pot (seatbelt): pot why, inside" sb_ pot why denied.example
  expect "answers from the run's folder" holds "denied.example"

  # processes: the run's group is stopped when the command exits, and what left it by setsid
  sb_ bash -c 'sleep 307 & python3 -I -c "import os; os.fork() or (os.setsid(), os.execvp(\"sleep\", [\"sleep\", \"308\"]))"' >/dev/null 2>&1
  sleep 0.5
  refuse "pot (seatbelt): survivors are stopped" pgrep -f "sleep 30[78]"
  # F2 (fix round 4): one that leaves the group and drops TMPDIR is still the run's (sandbox_check)
  sb_ python3 -I -c 'import os; os.fork() or (os.setsid(), os.execvpe("sleep", ["sleep", "309"], {k: v for k, v in os.environ.items() if k != "TMPDIR"}))' >/dev/null 2>&1
  sleep 0.5
  refuse "pot (seatbelt): F2, a survivor with no TMPDIR is stopped too" pgrep -f "sleep 309"
  # v2.20.2: a run's end leaves other sandboxed processes alone. A broad profile (allow default, as many of macOS's
  # own agents' may read the user's folders) reads the run's policy.json too; v2.20.0-v2.20.1 signalled them all
  # (neagent, so Tailscale; rapportd, secd …). One started before the run, one during it (only the profile tells)
  sandbox-exec -p '(version 1)(allow default)' sleep 311 >/dev/null 2>&1 &
  local by1=$!
  sb_ bash -c 'sleep 3' >/dev/null 2>&1 &
  local sbr=$!
  sleep 1
  sandbox-exec -p '(version 1)(allow default)' sleep 312 >/dev/null 2>&1 &
  local by2=$!
  wait "$sbr" || true
  sleep 0.5
  expect "pot (seatbelt): v2.20.2, a sandboxed process from before the run outlives its end" kill -0 "$by1"
  expect "pot (seatbelt): v2.20.2, and one started during the run" kill -0 "$by2"
  kill "$by1" "$by2" 2>/dev/null || true
  # F4: the folders pot makes on the way to a link are kept, as the link is: fixedtest's host memory, linked file by
  # file at ~/.claude/projects/KEY/memory in its state. fixedtest is sec_pot's: its grant and folders are made here,
  # since sec_pot's bubblewrap part never runs on a Mac
  local key && key=$(pwd -P | sed 's/[^A-Za-z0-9]/-/g')
  mkdir -p "$work/cfg/plugins" "$work/cfg/skills" "$work/cfg/projects/$key/memory"
  echo m > "$work/cfg/projects/$key/memory/m.md"
  ran "pot (seatbelt): pot grant fixed" tty_ $'y\n' "$work/sbr/bin/pot" grant fixed
  ran "pot (seatbelt): F4, the folders made for a link" sb_ fixedtest -c "cd ~/.claude && test -L projects/$key/memory/m.md && for d in projects projects/$key projects/$key/memory; do mv \"\$d\" \"\$d.moved\" 2>/dev/null && echo \"moved \$d\"; done; echo done"
  expect "can't be renamed" test "$out" = "done"
  ran "pot (seatbelt): at a terminal, the run is the foreground group" tty_ '' "$work/sbr/bin/pot" python3 -I -c '
import fcntl, os, termios
print("fg", os.tcgetpgrp(0) == os.getpgrp())
try:
    fcntl.ioctl(0, termios.TIOCSTI, b"x"); print("TIOCSTI allowed")
except OSError as e:
    print("TIOCSTI", e.errno)'
  expect "in the foreground" holds "fg True"
  expect "TIOCSTI is refused (no-tiocsti)" holds "TIOCSTI 1"

  # v2.20.0: a run starts no run inside it; a kit command's wrapper starts a sibling, under Seatbelt too. sbk-tool
  # asks for api.sbk.example (its kit's grant); a run with that host for one run covers it, so it starts as a sibling
  ran "pot (seatbelt): grant sbk" tty_ $'y\n' "$work/sbr/bin/pot" grant sbk
  ran "pot (seatbelt): explain, a run that covers sbk-tool" sb_ explain --net api.sbk.example bash
  expect "lists it as a sibling" holds "sbk-tool (sbk): sibling"
  # shellcheck disable=SC2016 # the run's own $?
  ran "pot (seatbelt): a sibling" sb_ --net api.sbk.example bash -c 'sbk-tool; sbk-tool fail; echo "exit=$?"'
  expect "runs" has "sbk-tool hi"
  expect "and its exit code comes back" has "exit=7"
  ran "pot (seatbelt): its record" bash -c "grep -h '\"via\":\"sibling\"' '$work'/sbstate/runs/*/log.jsonl | head -1"
  # shellcheck disable=SC2016 # bash -c's own $1
  expect "is a sibling's, by the host" bash -c 'jq -e "select(.by == \"host\" and .invoker == \"run\")" <<<"$1" >/dev/null' _ "$out"
  # shellcheck disable=SC2016 # the run's own $?
  ran "pot (seatbelt): ^C at the terminal" tty_ $'\003' "$work/sbr/bin/pot" --net api.sbk.example bash -c 'sbk-tool sleep 3200; echo "client-exit=$?"'
  expect "reaches the sibling" holds "sbk-tool: INT"
  expect "whose status comes back" holds "client-exit=130"
  ran "pot (seatbelt): the client killed" sb_ --net api.sbk.example bash -c 'sbk-tool sleep 3201 & sleep 5; kill -9 $!; sleep 5; echo done'
  expect "stops the sibling" test "$(pgrep -f 'sleep 3201' | wc -l | tr -d ' ')" = 0
  # the review: under Seatbelt the command is its own process group; one that ignores SIGTERM gets SIGKILL anyway
  ran "pot (seatbelt): a sibling that ignores SIGTERM, its client killed" sb_ --net api.sbk.example bash -c 'sbk-tool noterm 3203 & sleep 5; kill -9 $!; sleep 8; echo done'
  expect "is stopped too" test "$(pgrep -f 'sleep 3203' | wc -l | tr -d ' ')" = 0
  ran "pot (seatbelt): the caller's run ends" sb_ --net api.sbk.example bash -c 'sbk-tool sleep 3202 & sleep 5; echo leaving'
  sleep 6
  expect "stops the sibling with it" test "$(pgrep -f 'sleep 3202' | wc -l | tr -d ' ')" = 0
  # v2.20.1 (the v2.20.0 Mac log): a host pot killed by SIGTERM; the next start, once its backend has ended, clears
  # its children.sock (its only socket here: Seatbelt's proxy is TCP) and its tree's slots
  sb_ --net api.sbk.example bash -c 'sbk-tool >/dev/null; sleep 3204; true' >/dev/null 2>&1 &
  local kbg=$! krd="" kid="" kpid="" d
  for _ in $(seq 60); do
    for d in "$work"/sbstate/runs/*/; do
      [ -S "$d/children.sock" ] && [ -d "$work/sbstate/trees/$(basename "$d")" ] && krd=${d%/} && break 2
    done
    sleep 0.5
  done
  kid=$(basename "${krd:-none}") && kpid=$(jq -r 'select(.ev == "start") | .pid' "$krd/log.jsonl" 2>/dev/null || true)
  expect "pot (seatbelt): a run with a sibling has its socket and its tree's slots" test -n "$kpid"
  kill -TERM "$kpid" && { wait "$kbg" 2>/dev/null || true; }
  ran "pot (seatbelt): its host pot killed by SIGTERM, another run starts while its command runs" sb_ true
  expect "which leaves the live run's socket" test -S "$krd/children.sock"
  pkill -f 'sleep 3204' || true
  for _ in $(seq 30); do [ -e "$krd/status" ] && break; sleep 0.5; done
  expect "its backend ends" test -e "$krd/status"
  ran "pot (seatbelt): another run starts after it" sb_ true
  expect "and clears its children.sock" test ! -e "$krd/children.sock"
  expect "and its tree's slots" test ! -e "$work/sbstate/trees/$kid"
  refused "pot (seatbelt): pot inside a run" sb_ pot work true
  expect "is refused: no run inside a run" holds "pot inside a run starts no run"
  refused "pot (seatbelt): pot dispatch inside a run" sb_ pot dispatch true
  expect "is the host's" holds "pot (work): pot dispatch is the host's: a run starts no clone inside it; run it on the host"
  # the inside-run proof is sandbox_check, which a host process can't fake: a forged POT_POLICY is a host call
  mkdir -p "$work/sbfake" && echo '{"context":"forged"}' > "$work/sbfake/policy.json" && chmod 444 "$work/sbfake/policy.json"
  ran "pot (seatbelt): a forged POT_POLICY on the host" env POT_POLICY="$work/sbfake/policy.json" POT_QUIET=1 "$work/sbr/bin/pot" explain true
  expect "is the host's explain" holds "context    work: in-place, access write, seatbelt"

  # dispatch: the clone at its own path, its commits back through the sandboxed upload-pack
  ran "pot (seatbelt): a dispatch" sb_loud dispatch bash -c 'pwd; echo d > d.txt && git add d.txt && git commit -qm from-the-clone'
  expect "says the clone is at its own path" holds "clone at its own path"
  expect "runs in it" holds "/sbstate/runs/"
  local id && id=$(sb_ runs --json | jq -r '[.runs[] | select(.mode == "clone")][0].id')
  ran "pot (seatbelt): pot review" sb_ review "$id"
  expect "names the commit" holds "from-the-clone"
  ran "pot (seatbelt): pot review --fetch" sb_ review "$id" --fetch
  expect "fetches it" test "$(git_ log -1 --format=%s "pot-$id/main")" = from-the-clone
  # F1 (fix round 4): the clone's git ran under a profile with the run's last denies, in a session of its own
  expect "pot (seatbelt): F1, git.sb denies TIOCSTI" grep -q '(deny file-ioctl (ioctl-command' "$work/sbstate/runs/$id/git.sb"
  expect "and the fcntls" grep -qF '(deny system-fcntl (fcntl-command 80 110))' "$work/sbstate/runs/$id/git.sb"
  expect "pot (seatbelt): runs/1, every log line and record validates" runs_valid "$work/sbstate" sb_
  expect "pot (seatbelt): policy.json/1, every policy.json validates" policies_valid "$work/sbstate"
  expect "pot (seatbelt): launch.json/1, every launch.json validates" launches_valid "$work/sbstate"
}

sec_dispatch() {
  prov sbd && release sbd v1.0.0
  local c=$work/dc && mkdir -p "$c" && git_ -C "$c" init -q && cd "$c"
  cat > flake.nix <<EOF
{
  inputs.pot.url = "path:$pot";
  outputs = { pot, ... }: {
    packages = pot.inputs.nixpkgs.lib.genAttrs [ "$sys" ] (s: let pkgs = pot.inputs.nixpkgs.legacyPackages.\${s}; in {
      pot-kits = pot.lib.pot.kitsFor {
        inherit pkgs;
        self = { kit = { protect = [ "AGENTS.md" ]; }; file = "flake.nix (fixture)"; };
        files = [ "scions" ];
      };
    });
  };
}
EOF
  echo "# agents" > AGENTS.md && echo data > notes.md
  by_hand sbd && commit init && quiet nix flake lock && commit lock
  git_ remote add origin ssh://git@fake.example/dc.git
  local base && base=$(git_ rev-parse HEAD)

  local r=$work/droot && mkdir -p "$r" "$work/drootrepo" "$work/dbin"
  git_ -C "$work/drootrepo" init -q && printf 'programs.pot = {\n  a = 1;\n};\n' > "$work/drootrepo/pot.nix"
  git_ -C "$work/drootrepo" add pot.nix && git_ -C "$work/drootrepo" commit -q -m init
  cat > "$r/flake.nix" <<EOF
{
  inputs.pot.url = "path:$pot";
  inputs.nixpkgs.follows = "pot/nixpkgs";
  outputs = { pot, nixpkgs, ... }: let pkgs = nixpkgs.legacyPackages.$sys; bash = "\${pkgs.bashInteractive}/bin/bash";
    common = {
      inherit pkgs;
      name = "dispatch";
      grantsFile = "$work/dgrants/grants.json";
      stateDir = "$work/dstate";
      rootRepos = [ "$work/drootrepo" ];
      kits.needs.commands.needtest = { program = bash; requests = { network = [ "x.example" ];
        capabilities = [ { name = "status.relay"; } ]; }; };
      bindings."status.relay"."*" = { workmux = true; };
      bindings.nix."*" = { via = "daemon-untrusted"; };
      # L2: a kit never granted that no run waits for (pot grant with no argument names it, prompts for nothing)
      kits.unused.commands.unusedtest = { program = bash; requests.network = [ "u.example" ]; };
      bindings.credential."api.example" = { via = "file"; from.file = "$work/dtoken"; };
      bindings.credential."api.missing.example" = { via = "file"; from.file = "$work/dtoken-missing"; refresh = "make-token"; };
      contexts.strict = { extends = "work"; requires = [ "separate-kernel" ]; };
      limits = { memoryMax = "1G"; tasksMax = 512; };
    }; in {
    packages.$sys = {
      pot = pot.lib.mkRoot common;
      # the same state, clones kept 0 days and capped at 1 byte
      prune = pot.lib.mkRoot (common // { clones = { keepDays = 0; maxSize = "1"; }; });
      # G4: a stateDir with a space (the sandboxed upload-pack's path is quoted)
      spaced = pot.lib.mkRoot (common // { stateDir = "$work/d state"; });
      # K4: pot's state under what inspect reads
      inspectstate = pot.lib.mkRoot (common // { stateDir = "$work/dinsp/state"; inspect.home = [ "$work/dinsp" ]; });
    };
  };
}
EOF
  (cd "$r" && quiet nix flake lock)
  ran "dispatch: a root" nix build -o "$work/dr" "path:$r#packages.$sys.pot"
  ran "dispatch: a root with clones.keepDays and maxSize" nix build -o "$work/drp" "path:$r#packages.$sys.prune"
  if [ "$(uname)" = Darwin ]; then
    echo "skip dispatch: runs (bubblewrap's; the pot section's Seatbelt runs cover dispatch on macOS)"
    return 0
  fi
  local bw && bw=$(nix build --no-link --print-out-paths "$np#bubblewrap")/bin/bwrap
  if ! "$bw" --unshare-all --unshare-user --disable-userns --ro-bind / / true 2>/dev/null; then
    echo "skip dispatch: runs (bubblewrap can't create an unprivileged user namespace here)"
    cd "$work"; return 0
  fi
  # shellcheck disable=SC2317 # run through ran, expect and refuse
  dl() { env -u CLAUDECODE -u CODEX_SANDBOX -u POT -u TMUX -u TMUX_PANE "$work/dr/bin/pot" "$@" </dev/null; }
  # shellcheck disable=SC2317
  d_() { POT_QUIET=1 dl "$@"; }
  # shellcheck disable=SC2317
  rec() { d_ runs "$1" --json | jq -r "$2"; }
  local short && short=$(git_ rev-parse --short HEAD)
  local st=$work/dstate/runs

  # X6: before any run, pot review says so, with no header
  ran "dispatch: X6, pot review with nothing to review" d_ review
  expect "says so" has "pot: no runs need review"
  refuse "with no header" holds "RUN "
  refused "dispatch: X6, --fetch with no run" d_ review --fetch
  expect "one pot:" holds "pot: review RUN --fetch|--discard: name the run (pot review lists them)"
  refuse "never two" holds "pot: pot review"

  # a run in a clone: at the original path, from the base, with POT_CLONE; the source untouched
  echo dirty >> notes.md
  # shellcheck disable=SC2016 # the run's shell expands them
  ran "dispatch: a run in a clone" dl dispatch bash -c 'pwd; echo "clone=$POT_CLONE"; grep -c dirty notes.md || true
    echo new > made.txt && git add made.txt && git commit -qm "run: made" && git log -1 --format=%s
    touch .git/hooks/x 2>/dev/null || echo "hooks: read-only"; git config core.fsmonitor x 2>/dev/null || echo "config: read-only"
    echo .. > .git/commondir 2>/dev/null || echo "commondir: refused"'
  expect "at the original path" has "$c"
  expect "POT_CLONE=1" has "clone=1"
  expect "from the base, not the working tree" has "0"
  expect "the commit is made in the clone" has "run: made"
  expect "the clone's hooks are read-only" has "hooks: read-only"
  expect "S1: the clone's absent git names can't be created" has "commondir: refused"
  expect "and its config" has "config: read-only"
  expect "the dirty-tree notice" holds "pot (dispatch): the clone starts at main ($short); your 1 uncommitted change stays here, outside it"
  expect "the startup line names the base" holds "pot: dispatch · dc · from main ($short)"
  local rid && rid=$(sed -n 's/^pot: run \([0-9a-f]\{4\}\) · 1 commit on main: .*/\1/p' <<<"$out")
  expect "the exit line names pot review" holds "pot: run $rid · 1 commit on main: pot review $rid, then pot review $rid --fetch"
  expect "nothing in the source tree" test ! -e made.txt
  expect "nor in its history" test "$(git_ rev-parse HEAD)" = "$base"
  expect "the clone is in pot's state" test -d "$st/$rid/clone/.git"
  expect "its scion too" test -f "$st/$rid/clone/scions/sbd/README.md"
  expect "staged at the run's end" test -d "$st/$rid/staging/top.git"
  git_ checkout -q -- notes.md

  # runs/1: the record, pot runs, pot review
  ran "dispatch: pot runs ID --json" d_ runs "$rid" --json
  expect "state review" test "$(jq -r .state <<<"$out")" = review
  expect "one commit" test "$(jq -r .clone.commits <<<"$out")" = 1
  expect "from the base" test "$(jq -r .clone.base <<<"$out")" = "$base"
  expect "its context and mode" test "$(jq -r '"\(.context) \(.mode)"' <<<"$out")" = "dispatch clone"
  ran "dispatch: pot runs" d_ runs
  expect "lists it" holds "$rid  review   dispatch"
  ran "dispatch: pot runs ID" d_ runs "$rid"
  expect "says what is next" holds "next       pot review $rid · pot review $rid --fetch"
  ran "dispatch: pot review" d_ review
  expect "lists it" holds "$rid  review"
  ran "dispatch: pot review ID" d_ review "$rid"
  expect "X6: its next names --fetch, not itself" holds "next       pot review $rid --fetch"
  refuse "X6: never pot review ID itself" holds "next       pot review $rid ·"
  expect "shows its commit, read from staging" holds "run: made"
  refused "dispatch: pot runs, an unknown run" d_ runs ffff
  expect "says so" holds "pot runs: no run ffff in pot's state (pot runs lists them)"

  # fetch-back
  ran "dispatch: pot review ID --fetch" dl review "$rid" --fetch
  expect "says what it fetched" holds "pot-$rid/main (1 commit on $short)"
  expect "and how to take it" holds "  take it: git merge pot-$rid/main"
  expect "into refs/remotes/pot-ID/*" test "$(git_ log -1 --format=%s "refs/remotes/pot-$rid/main")" = "run: made"
  expect "the run is exited, fetched" test "$(rec "$rid" '"\(.state) \(.clone.fetched[0])"')" = "exited refs/remotes/pot-$rid/main"
  expect "and no longer needs review" test -z "$(d_ review | grep "^$rid ")"
  ran "dispatch: --discard" dl review "$rid" --discard
  expect "says so" holds "pot: discarded run $rid's clone"
  expect "the fetched refs stay" holds "  the fetched refs stay: git branch -r -d pot-$rid/main"
  expect "the clone is gone" test ! -e "$st/$rid/clone"
  expect "its log stays" test -f "$st/$rid/log.jsonl"
  # F4/R1: a clone holding a read-only folder (so the first removal pass fails) and links to host folders: removal
  # changes no mode outside the clone, by --discard, keepDays or maxSize
  mkdir -p "$work/outside/sub" && chmod 755 "$work/outside" "$work/outside/sub"
  # shellcheck disable=SC2016 # the run's shell expands them
  ran "dispatch: F4, a clone with links out and a read-only folder" d_ dispatch bash -c 'mkdir -p ro && touch ro/f &&
    ln -s "$1" ro/out && ln -s "$1/sub" outlink && chmod 500 ro && git commit -q --allow-empty -m f4' _ "$work/outside"
  local f4 && f4=$(d_ runs --json | jq -r '[.runs[] | select(.state == "review")][0].id')
  ran "dispatch: F4, --discard" dl review "$f4" --discard
  expect "F4: the clone is gone" test ! -e "$st/$f4/clone"
  expect "F4: the linked host folder keeps its mode" test "$(stat -c %a "$work/outside")" = 755
  expect "F4: and its subfolder" test "$(stat -c %a "$work/outside/sub")" = 755
  ran "dispatch: pot runs, after --discard" d_ runs --all
  expect "says discarded" grep -q "^$rid .*discarded$" <<<"$out"
  refused "dispatch: --fetch after --discard" d_ review "$rid" --fetch
  expect "says so" holds "pot: run $rid's clone was discarded"

  # --from, kits from the base
  git_ checkout -q -b side && echo s > side.txt && commit side && git_ checkout -q main
  ran "dispatch: --from a branch" d_ dispatch --from side bash -c 'git branch --show-current; test -f side.txt && echo has-side'
  expect "on that branch" has side
  expect "at its commit" has has-side
  local fr && fr=$(d_ runs --json | jq -r '.runs[0].id')
  expect "F1: a --from run with no commit of its own is not in review" test "$(rec "$fr" '"\(.state) \(.clone.commits)"')" = "exited 0"
  ran "dispatch: F1, the clone's branches" d_ dispatch --from side bash -c 'git for-each-ref --format="%(refname)" refs/heads'
  expect "F1: only the base's branch" has "refs/heads/side"
  refuse "F1: no other local branch" has "refs/heads/main"
  sed -i 's/protect = \[ "AGENTS.md" \]/protect = [ "AGENTS.md" "notes.md" ]/' flake.nix
  refused "dispatch: in place, the working tree's kit protects notes.md" d_ bash -c 'echo x >> notes.md'
  ran "dispatch: the base's kit does not" d_ dispatch bash -c 'echo x >> notes.md && echo wrote'
  expect "(kits from the base)" has wrote
  git_ checkout -q -- flake.nix

  # --detach: the id alone on stdout; pot runs ID --follow waits for the end
  # shellcheck disable=SC2016
  ran "dispatch: --detach" d_ dispatch --detach bash -c 'echo bg > bg.txt && git add bg.txt && git commit -qm bg && sleep 1'
  local did=$out
  expect "prints the id alone" grep -qx '[0-9a-f]\{4\}' <<<"$did"
  expect "running" test "$(rec "$did" .state)" = running
  ran "dispatch: pot runs ID --follow" timeout 120 env -u POT POT_QUIET=1 "$work/dr/bin/pot" runs "$did" --follow
  expect "follows it to its end" holds "exit 0"
  expect "then review" test "$(rec "$did" '"\(.state) \(.detached) \(.clone.commits)"')" = "review true 1"
  expect "its output kept" test -f "$st/$did/output.log"

  # pot dies before after (kill -9 of the detached pot's group): the next pot runs completes the staging,
  # through the sandboxed upload-pack, whatever hooks and config the clone holds
  # shellcheck disable=SC2016
  ran "dispatch: a detached run to kill" d_ dispatch --detach bash -c 'echo c > c.txt && git add c.txt && git commit -qm crash && sleep 300'
  local kid=$out pgid
  for _ in $(seq 1 100); do git_ -C "$st/$kid/clone" log -1 --format=%s 2>/dev/null | grep -qx crash && break; sleep 0.2; done
  pgid=$(jq -r 'select(.ev == "start") | .pid' "$st/$kid/log.jsonl")
  kill -9 -- "-$pgid" 2>/dev/null || true
  for _ in $(seq 1 50); do ! kill -0 "$pgid" 2>/dev/null && break; sleep 0.1; done
  local h
  for h in pre-receive post-receive reference-transaction post-checkout fsmonitor-watchman pre-auto-gc post-index-change; do
    printf '#!/bin/sh\ntouch %s\n' "$work/PLANTED-hook-$h" > "$st/$kid/clone/.git/hooks/$h"
    chmod +x "$st/$kid/clone/.git/hooks/$h"
  done
  git_ -C "$st/$kid/clone" config core.fsmonitor "touch $work/PLANTED-fsmonitor"
  git_ -C "$st/$kid/clone" config uploadpack.packObjectsHook "touch $work/PLANTED-packobjectshook"
  git_ -C "$st/$kid/clone" config core.sshCommand "touch $work/PLANTED-sshcommand"
  mkdir -p "$st/$kid/clone/nested" && git_ -C "$st/$kid/clone/nested" init -q
  git_ -C "$st/$kid/clone/nested" config core.fsmonitor "touch $work/PLANTED-nested"
  # R9, G1: a recovery another pot holds (its lock) is left to that pot; a reader waits for it (15s), then rereads
  flock "$st/$kid/recover.lock" sleep 20 &
  local hold=$!
  sleep 0.5
  dl runs "$kid" > "$work/g1-text.out" 2>&1 &
  local g1t=$!
  ran "dispatch: G1, pot runs while another holds the recovery lock past the wait" d_ runs "$kid" --json
  expect "G1: shows recovering, never a run with no commits" test "$(jq -r '"\(.recovering) \(.state)"' <<<"$out")" = "true running"
  wait "$g1t" || true
  out=$(cat "$work/g1-text.out")
  expect "G1: and says so" holds "recovering: another pot is completing this run's staging; pot runs $kid again"
  refuse "G1: not lost" holds "unknown: pot ended"
  wait "$hold" || true
  expect "R9: is not recovered while held" test "$(grep -c '"ev":"after"' "$st/$kid/log.jsonl")" = 0
  flock "$st/$kid/recover.lock" sleep 2 &
  hold=$!
  sleep 0.5
  ran "dispatch: G1, pot runs while the lock is held for 2s" d_ runs "$kid" --json
  wait "$hold" || true
  expect "G1: waits, then completes the staging itself: review" test "$(jq -r '"\(.state) \(.clone.commits)"' <<<"$out")" = "review 1"
  expect "R9: recovered once" test "$(grep -c '"ev":"after"' "$st/$kid/log.jsonl")" = 1
  ran "dispatch: pot runs ID after the kill" d_ runs "$kid" --json
  expect "completes the staging: review" test "$(jq -r '"\(.state) \(.lost) \(.clone.commits)"' <<<"$out")" = "review true 1"
  ran "dispatch: pot runs lists it" d_ runs
  expect "in review, though its pot was killed" holds "$kid  review"
  ran "dispatch: and --fetch" d_ review "$kid" --fetch
  expect "fetches its commit" test "$(git_ log -1 --format=%s "refs/remotes/pot-$kid/main")" = crash
  expect "no hook or config of the clone ran on the host" test -z "$(find "$work" -maxdepth 1 -name 'PLANTED-*')"
  expect "the planted nested repository reached nothing" test ! -e nested
  git_ -C "$st/$kid/clone" status --short >/dev/null 2>&1 || true
  expect "(control: plain host git in the clone runs its fsmonitor)" test -e "$work/PLANTED-fsmonitor"
  rm -f -- "$work"/PLANTED-*

  # a run whose pot is killed in place: exited, lost
  setsid env -u CLAUDECODE -u CODEX_SANDBOX -u POT POT_QUIET=1 "$work/dr/bin/pot" bash -c 'sleep 300' </dev/null >/dev/null 2>&1 &
  local lp=$! lid=
  for _ in $(seq 1 100); do lid=$(grep -l "\"pid\":$lp," "$st"/*/log.jsonl 2>/dev/null | head -1) && [ -n "$lid" ] && break; sleep 0.2; done
  lid=$(basename "$(dirname "$lid")")
  expect "dispatch: a run in place is running" test "$(rec "$lid" .state)" = running
  kill -9 -- "-$lp" 2>/dev/null || true
  wait "$lp" 2>/dev/null || true
  expect "killed: exited, lost" test "$(rec "$lid" '"\(.state) \(.lost) \(.exit)"')" = "exited true null"

  # R3: a host-run file under a non-ASCII folder, changed by a run's commit, is flagged (git paths read with -z)
  mkdir -p "dossier-é" && echo "x:" > "dossier-é/justfile" && commit "a justfile under dossier-é"
  # shellcheck disable=SC2016
  ran "dispatch: R3, a commit changing dossier-é/justfile" d_ dispatch bash -c 'b=$(echo "y:" | git hash-object -w --stdin)
    git update-index --cacheinfo 100644,"$b",dossier-é/justfile && git commit -qm "change the justfile"'
  local r3 && r3=$(d_ runs --json | jq -r '.runs[0].id')
  expect "R3: ATTENTION names it, unquoted" test "$(rec "$r3" '.attention | index("host-run: dossier-é/justfile")')" != null
  # G2: git's -z output read as bytes: a CR stays \x0d, a byte that is not UTF-8 shows as \xNN
  # shellcheck disable=SC2016 # the run's shell expands them
  ran "dispatch: G2, a commit changing justfiles under folders with a CR and a 0xff byte" dl dispatch bash -c '
    for d in "$(printf "cr\rdir")" "$(printf "bad\377dir")"; do mkdir -p "$d" && echo "x:" > "$d/justfile" && git add -- "$d/justfile"; done
    git commit -qm g2'
  # (the run's id from its exit line: runs started in the same second have no newest)
  local g2 && g2=$(sed -n 's/^pot: run \([0-9a-f]\{4\}\) · .*/\1/p' <<<"$out" | tail -1)
  expect "G2: the CR stays \\x0d" test "$(rec "$g2" '.attention | index("host-run: bad\\xffdir/justfile, cr\\x0ddir/justfile")')" != null
  # F2: a dispatch that stops with 125 (needs is not granted yet) records its clone; --discard removes it
  refused "dispatch: F2, a dispatch that stops (125)" d_ dispatch needtest -c true
  local f2 && f2=$(d_ runs --json | jq -r '[.runs[] | select(.state == "stopped" and .mode == "clone")][0].id')
  expect "F2: its record has the clone" test "$(rec "$f2" '.clone.base | length')" = 40
  ran "dispatch: D4, pot runs ID of it" d_ runs "$f2"
  expect "D4: next names the fix, its kept clone and --discard" grep -qE "^next       pot grant needs · its clone is kept until [0-9]{4}-[0-9]{2}-[0-9]{2}: pot review $f2 --discard$" <<<"$out"
  ran "dispatch: F2, --discard of it" dl review "$f2" --discard
  expect "F2: says it discarded the clone" holds "pot: discarded run $f2's clone"
  expect "F2: the clone is gone" test ! -e "$st/$f2/clone"
  # a dispatch refused for another reason after its clone was made: no clone is left without a log
  refused "dispatch: F2, a dispatch of a program nowhere" d_ dispatch no-such-program-x
  expect "F2: no orphan clone" test -z "$(for d in "$st"/*/; do [ -d "$d/clone" ] && [ ! -s "$d/log.jsonl" ] && echo "$d"; done)"
  expect "D1: its fix names dispatch" holds "  one run:   pot dispatch --package PKG …"
  expect "C1: and no run folder without a log" test -z "$(for d in "$st"/*/; do [ -e "$d/log.jsonl" ] || echo "$d"; done)"
  refused "dispatch: C1, an in-place run refused before its start" d_ --write "$work/drootrepo" true
  expect "C1: leaves no run folder either" test -z "$(for d in "$st"/*/; do [ -e "$d/log.jsonl" ] || echo "$d"; done)"
  # D1: a dispatch's hints name dispatch: the denials file, the exit line, the not-found handler
  # shellcheck disable=SC2016 # the run's shell expands them
  local ask='exec 3<>/dev/tcp/127.0.0.1/3128 && printf "CONNECT %s HTTP/1.1\r\nHost: %s\r\n\r\n" "$1" "$1" >&3 && head -1 <&3'
  ran "dispatch: D1, a host not granted (with --net, so the run has a proxy)" dl dispatch --net allowed.example bash -c "$ask; cat /run/pot/denials; no-such-tool-d1; true" _ denied.example:443
  expect "D1: the denials file's one-run form is pot dispatch" holds "pot (dispatch): the run tried denied.example:443 (not in this run)."
  expect "D1: one run: pot dispatch --net" test "$(grep -c '^  one run:   pot dispatch --net denied.example …$' <<<"$out")" -ge 2
  refuse "D1: never pot --net (a work run)" holds "  one run:   pot --net"
  expect "D1: the not-found handler too" holds "  one run:   pot dispatch --package PKG …"
  refuse "D1: never pot --package" holds "  one run:   pot --package"

  # 125: a stopped record, and the pane workmux shows waiting
  # shellcheck disable=SC2016 # the stub's shell expands them
  printf '#!/bin/sh\necho "$TMUX_PANE $*" >> %s\n' "$work/dworkmux.log" > "$work/dbin/workmux" && chmod +x "$work/dbin/workmux"
  refused "dispatch: a 125 in a workmux pane" env PATH="$work/dbin:$PATH" TMUX=/tmp/t,1,0 TMUX_PANE=%7 POT_QUIET=1 \
    env -u CLAUDECODE -u CODEX_SANDBOX -u POT "$work/dr/bin/pot" needtest -c true </dev/null
  local sid && sid=$(d_ runs --json | jq -r '[.runs[] | select(.state == "stopped")][0].id')
  expect "writes a stopped record" test "$(rec "$sid" '"\(.stop.reason) \(.exit) \(.stop.fix)"')" = "grant 125 pot grant needs"
  expect "and sets the pane waiting" grep -qx '%7 set-window-status waiting' "$work/dworkmux.log"
  refused "dispatch: a 125 outside tmux" env PATH="$work/dbin:$PATH" POT_QUIET=1 env -u TMUX -u TMUX_PANE -u CLAUDECODE -u POT \
    "$work/dr/bin/pot" needtest -c true </dev/null
  expect "touches no pane" test "$(wc -l < "$work/dworkmux.log")" = 1
  refused "dispatch: a missing guarantee" env PATH="$work/dbin:$PATH" TMUX=/tmp/t,1,0 TMUX_PANE=%8 POT_QUIET=1 \
    env -u CLAUDECODE -u CODEX_SANDBOX -u POT "$work/dr/bin/pot" strict true </dev/null
  expect "exits 125 too" holds "requires separate-kernel"
  sid=$(d_ runs --json | jq -r '[.runs[] | select(.state == "stopped" and .context == "strict")][0].id')
  expect "a stopped record (guarantee)" test "$(rec "$sid" .stop.reason)" = guarantee
  expect "no pane: the run holds none (no status.relay)" test "$(wc -l < "$work/dworkmux.log")" = 1

  # the resource scope: runs without a terminal, in a user systemd scope with the root's limits
  if systemd-run --user --scope --quiet true 2>/dev/null; then
    d_ bash -c 'sleep 4.3217' &
    local bp=$! sp=
    for _ in $(seq 1 100); do sp=$(pgrep -f '^sleep 4.3217' | head -1) && [ -n "$sp" ] && break; sleep 0.1; done
    local cg && cg=$(cut -d: -f3 "/proc/$sp/cgroup")
    expect "dispatch: a run without a terminal is in a scope of its own (systemd-run's)" grep -q '/run-[^/]*\.scope$' <<<"$cg"
    expect "MemoryMax from the root" test "$(cat "/sys/fs/cgroup$cg/memory.max")" = 1073741824
    expect "TasksMax from the root" test "$(cat "/sys/fs/cgroup$cg/pids.max")" = 512
    wait "$bp"
    local pr && pr=$(grep -l '4.3217' "$st"/*/log.jsonl | head -1)
    expect "and proves resource-limits" jq -e '.backend.proved | index("resource-limits")' "$(dirname "$pr")/policy.json"
    # no systemd-run on PATH: a repository with no flake, so pot needs no nix either
    mkdir -p "$work/dplain" && git_ -C "$work/dplain" init -q && git_ -C "$work/dplain" commit -q --allow-empty -m init
    # shellcheck disable=SC2016 # the inner bash expands them
    ran "dispatch: with no user systemd" bash -c 'cd "$1" && env -u POT -u CLAUDECODE PATH="$2" "$3" true </dev/null' _ \
      "$work/dplain" "$work/dbin" "$work/dr/bin/pot"
    expect "the run starts, and says so" holds "no resource limits (no user systemd: pot doctor)"
  else
    echo "skip dispatch: resource scope (no user systemd here)"
  fi

  # S2: a run whose commits fail the staging fetch (fsck refuses a malformed commit): ATTENTION says so, the run stays
  # in review, and keepDays and maxSize never remove its clone
  # shellcheck disable=SC2016 # the run's shell expands them
  ran "dispatch: S2, a clone whose fetch fails" dl dispatch bash -c 'git commit -q --allow-empty -m good-one
    printf "tree 0000\nnot a commit\n" > /tmp/bad
    c=$(git hash-object -w --literally -t commit /tmp/bad) && echo "$c" > .git/refs/heads/bad && echo made-bad'
  expect "N1: the exit line names the failure, then the good ref's fetch" grep -qE "^pot: run [0-9a-f]{4} · 1 commit on main · fetch failed: refs/heads/bad: .*: pot review [0-9a-f]{4}, then pot review [0-9a-f]{4} --fetch$" <<<"$out"
  local fid && fid=$(d_ runs --json | jq -r '[.runs[] | select(.attention | any(startswith("fetch failed")))][0].id')
  expect "S2: ATTENTION says the fetch failed" test -n "$fid" -a "$fid" != null
  expect "N2: the record carries clone.fetchFailed" test "$(rec "$fid" '.clone.fetchFailed | startswith("refs/heads/bad")')" = true
  expect "S2: the run is in review" test "$(rec "$fid" .state)" = review
  ran "dispatch: S2, pot review lists it" d_ review
  expect "with the failure" grep -q "^$fid .*fetch failed" <<<"$out"
  expect "F3/R2: the good branch is staged all the same" test "$(rec "$fid" .clone.commits)" = 1
  expect "F3/R2: the failure names the branch" test "$(rec "$fid" '.attention[] | select(startswith("fetch failed"))' | grep -c 'refs/heads/bad')" = 1
  expect "F8: and git's own error, not its last line" test "$(rec "$fid" '.attention[] | select(startswith("fetch failed"))' | grep -c 'bogus commit object')" = 1
  # the second trigger: a clone whose git directory lost its objects (fsck or upload-pack fails for every ref)
  # shellcheck disable=SC2016
  ran "dispatch: F3, a clone whose objects are gone" dl dispatch bash -c 'git commit -q --allow-empty -m lost && rm -rf .git/objects/??'
  expect "N1: the exit line names the failure and pot review, not no commits" grep -qE "^pot: run [0-9a-f]{4} · fetch failed: .*: pot review [0-9a-f]{4}$" <<<"$out"
  refuse "N1: never no commits … --discard" holds "no commits; the clone is kept"
  # (the run with no commit staged: two runs started in the same second sort either way)
  local f3b && f3b=$(d_ runs --json | jq -r '[.runs[] | select((.attention | any(startswith("fetch failed"))) and .clone.commits == 0)][0].id')
  ran "dispatch: N1, pot runs ID of it" d_ runs "$f3b"
  expect "N1: next names the failure, no --fetch" has "next       pot review $f3b · pot review $f3b --discard (the staging fetch failed: nothing to fetch)"
  refuse "N1: never --fetch" holds "--fetch"
  expect "F3: it is in review too" test "$(rec "$f3b" .state)" = review
  expect "F3: a different run from the first" test "$f3b" != "$fid"
  # K1: a branch whose name is not UTF-8 (b\xfe) beside a commit on main: both staged, the run in review, main fetched
  # shellcheck disable=SC2016 # the run's shell expands them
  ran "dispatch: K1, a commit on main and a branch b\\xfe" dl dispatch bash -c 'git commit -q --allow-empty -m k1-main && git branch "$(printf "b\376")"'
  local k1 && k1=$(sed -n 's/^pot: run \([0-9a-f]\{4\}\) · .*/\1/p' <<<"$out" | tail -1)
  expect "K1: the exit line names the branch, escaped" holds "1 commit on main, 1 on b\\xfe: pot review $k1"
  expect "K1: the run is in review" test "$(rec "$k1" '"\(.state) \(.clone.commits)"')" = "review 1"
  ran "dispatch: K1, its --fetch" dl review "$k1" --fetch
  expect "K1: main is fetched" test "$(git_ log -1 --format=%s "refs/remotes/pot-$k1/main")" = k1-main
  expect "K1: the fetch names the branch, escaped" holds "pot-$k1/b\\xfe (1 commit on"
  expect "K1: its next steps name the base's branch first" grep -qE "^  review:  git log --stat [0-9a-f]{7}\.\.pot-$k1/main$" <<<"$out"
  # K1: an after step that fails for any reason (the tests' fault point) is a fetch failure: review, kept
  ran "dispatch: K1, an after step that fails" env -u CLAUDECODE -u CODEX_SANDBOX -u POT -u TMUX -u TMUX_PANE POT_TEST_FAULT=after "$work/dr/bin/pot" dispatch bash -c 'git commit -q --allow-empty -m k1-fault' </dev/null
  local kf && kf=$(sed -n 's/^pot: run \([0-9a-f]\{4\}\) · .*/\1/p' <<<"$out" | tail -1)
  expect "K1: the exit line says the look failed" grep -qE "^pot: run $kf · fetch failed: the host's look failed \(RuntimeError: an injected fault \(POT_TEST_FAULT=after\)\): pot review $kf$" <<<"$out"
  expect "K1: the run is in review" test "$(rec "$kf" .state)" = review
  expect "K1: the backend's own event says so (the denials and the rest kept)" test "$(jq -s '[.[] | select(.ev == "after")][0] | has("error")' "$st/$kf/log.jsonl")" = false
  expect "K1: with clone.fetchFailed" test "$(rec "$kf" '.clone.fetchFailed | startswith("the host'"'"'s look failed")')" = true
  ran "dispatch: K1, a backend that prints no after event" env -u CLAUDECODE -u CODEX_SANDBOX -u POT -u TMUX -u TMUX_PANE POT_TEST_FAULT=after-exit "$work/dr/bin/pot" dispatch bash -c 'git commit -q --allow-empty -m k1-exit' </dev/null
  local ke && ke=$(sed -n 's/^pot: run \([0-9a-f]\{4\}\) · .*/\1/p' <<<"$out" | tail -1)
  expect "K1: is a failed look too: review" test "$(rec "$ke" '"\(.state) \(.clone.fetchFailed)"')" = "review the host's look failed (exit 3)"
  ran "dispatch: K1, pot review" d_ review
  expect "K1: lists it" grep -q "^$kf .*fetch failed: the host's look failed" <<<"$out"
  # K2: a fetched branch named with U+202E and a C1 control: escaped in pot runs' clone row
  # shellcheck disable=SC2016
  ran "dispatch: K2, a branch named with U+202E and U+0085" dl dispatch bash -c 'git commit -q --allow-empty -m k2 && git branch "$(printf "k2\342\200\256x\302\205y")"'
  local k2 && k2=$(sed -n 's/^pot: run \([0-9a-f]\{4\}\) · .*/\1/p' <<<"$out" | tail -1)
  ran "dispatch: K2, its --fetch" d_ review "$k2" --fetch
  ran "dispatch: K2, pot runs ID" d_ runs "$k2"
  expect "K2: fetched: escapes the name" holds "pot-$k2/k2\\u202ex\\x85y"
  refuse "K2: never the raw override" holds "$(printf '\342\200\256')"
  # F4/R1 by prune: a no-commit clone with a link out and a read-only folder, removed by keepDays 0
  mkdir -p "$work/outside2" && chmod 755 "$work/outside2"
  # shellcheck disable=SC2016
  ran "dispatch: F4, a no-commit clone with a link out" d_ dispatch bash -c 'mkdir -p ro2 && touch ro2/f && ln -s "$1" ro2/out && chmod 500 ro2' _ "$work/outside2"
  # clones kept: keepDays and maxSize discard only fetched, discarded or commit-less clones
  ran "dispatch: a start with keepDays 0 and maxSize 1" env -u POT -u CLAUDECODE "$work/drp/bin/pot" dispatch true </dev/null
  expect "S2: keepDays keeps the clone whose fetch failed" test -d "$st/$fid/clone"
  expect "K1: and the one whose look failed" test -d "$st/$kf/clone"
  expect "F3: and the second one" test -d "$st/$f3b/clone"
  expect "F4: prune left the linked host folder's mode" test "$(stat -c %a "$work/outside2")" = 755
  expect "keeps the unfetched one" test -d "$st/$did/clone"
  expect "discards a fetched one" test ! -e "$st/$kid/clone"
  expect "names the oldest still in review" holds "pot review $did --discard"
  expect "logs why" jq -e 'select(.ev == "discard" and .why == "keepDays")' "$st/$kid/log.jsonl"

  # the narrowed repo-kit source (ruling 1): grants/1 is unchanged, a powerful grant still records the flake
  # directory's tree, and it holds while flake.nix, flake.lock and the kit as built are unchanged
  # shellcheck disable=SC2317
  tty_() { local a=$1; shift; (sleep 2; printf '%s' "$a") | script -qec "env -u CLAUDECODE -u CODEX_SANDBOX -u POT -u TMUX $*" /dev/null; }
  local n=$work/dn && mkdir -p "$n" && git_ -C "$n" init -q && cd "$n"
  cat > flake.nix <<EOF
{
  inputs.pot.url = "path:$pot";
  outputs = { pot, ... }: {
    packages.$sys.pot-kits = pot.lib.pot.kitsFor {
      pkgs = pot.inputs.nixpkgs.legacyPackages.$sys;
      self = { kit = import ./kit.nix; file = "flake.nix (fixture)"; };
      files = [ "scions" ];
    };
  };
}
EOF
  printf '{ requests.capabilities = [ { name = "nix"; } ]; commands.dn-tool.program = ./dn-tool.sh; }\n' > kit.nix
  printf '#!/bin/sh\necho dn-tool 1\n' > dn-tool.sh && chmod +x dn-tool.sh
  echo "test 1" > t.test.ts
  by_hand sbd && commit init && quiet nix flake lock && commit lock
  git_ remote add origin ssh://git@fake.example/dn.git
  ran "dispatch: grant a repo kit's powerful item" tty_ $'y\n' "$work/dr/bin/pot" grant dn
  expect "it records the tree" test "$(jq -r '.grants["repo:fake.example/dn"].source.tree' "$work/dgrants/grants.json")" = "$(git_ rev-parse 'HEAD^{tree}')"
  ran "dispatch: a run, the grant as written" d_ true
  local gsha && gsha=$(sha256sum "$work/dgrants/grants.json")
  echo "test 2" > t.test.ts && commit "a test file"
  ran "dispatch: a test file changed: the grant holds" d_ true
  release sbd v1.0.1 && git_ -C scions/sbd fetch -q --tags && git_ -C scions/sbd checkout -q --detach v1.0.1 && git_ add scions/sbd && commit "sbd v1.0.1"
  ran "dispatch: a scion bumped: the grant holds" d_ true
  expect "and the grants file is untouched (its stored form)" test "$(sha256sum "$work/dgrants/grants.json")" = "$gsha"
  printf '#!/bin/sh\necho dn-tool 2\n' > dn-tool.sh && commit "the kit's command changed"
  refused "dispatch: the kit as built changed" d_ true
  expect "re-asks: changed source" holds "dn (repo:fake.example/dn) changed source since its grant:"
  expect "saying the kit changed" holds "the kit as built changed"
  git_ revert --no-edit HEAD >/dev/null
  ran "dispatch: reverted: the grant holds again" d_ true
  printf '\n' >> flake.lock && commit "flake.lock changed"
  local wl0 && wl0=$(wc -l < "$work/dworkmux.log")
  refused "dispatch: flake.lock changed: re-asks" env PATH="$work/dbin:$PATH" TMUX=/tmp/t,1,0 TMUX_PANE=%9 POT_QUIET=1 \
    env -u CLAUDECODE -u CODEX_SANDBOX -u POT "$work/dr/bin/pot" true </dev/null
  expect "F6/R6: a 125 of a run that holds no pane (true asks no status.relay) sets none" test "$(wc -l < "$work/dworkmux.log")" = "$wl0"
  expect "with its diffstat" holds "flake.lock | 1 +"
  git_ revert --no-edit HEAD >/dev/null
  echo "test 3" > t.test.ts && commit "another test file"
  rm -rf -- "$work/dstate/cache/sources"
  refused "dispatch: with no record of the kit as built (another machine's grant): the tree alone decides" d_ true
  expect "changed source" holds "changed source since its grant:"
  cd "$c"

  # pot grant with no argument: every stale kit here, and every kit a stopped run waits for, wherever it ran
  # L2: it prompts only for kits a stopped run waits for and stale grants, stopped ones first (by name: dn, then
  # needs); a kit never granted that no run waits for is named after the prompts
  ran "dispatch: pot grant, no argument, from another repository" tty_ $'y\nn\n' "$work/dr/bin/pot" grant
  expect "offers the stopped run's kit" holds "dn   repo kit (repo:fake.example/dn), for every run in this repository   changed source"
  expect "L2: and the stopped root kit" holds "needs   root kit (root:needs), for every run that uses it   not granted"
  refuse "L2: not the kit no run waits for" holds "unused   root kit"
  expect "L2: which it names after the prompts" holds "pot: also not granted: unused (pot grant NAME)"
  expect "naming the stopped run and where" grep -qE "\(stopped run [0-9a-f]{4}, in $n\)" <<<"$out"
  (cd "$n" && ran "dispatch: then that repository's run goes on" d_ true)
  # pot grant --commit (decision 3: opt-in each call): commits the grants file's path alone, never pushes
  git_ -C "$work/dgrants" init -q && git_ -C "$work/dgrants" add grants.json && git_ -C "$work/dgrants" commit -q -m grants
  echo other > "$work/dgrants/other.txt" && git_ -C "$work/dgrants" add other.txt
  ran "dispatch: pot grant --commit" tty_ $'y\n' "$work/dr/bin/pot" grant --commit needs
  expect "says it committed, and does not push" grep -qE "committed $work/dgrants/grants.json \([0-9a-f]{7}\); push it with $work/dgrants's next push" <<<"$out"
  expect "the commit holds the grants file alone" test "$(git_ -C "$work/dgrants" show --name-only --format= HEAD)" = grants.json
  expect "its subject names the kit" test "$(git_ -C "$work/dgrants" log -1 --format=%s)" = "pot grant: needs"
  expect "another staged file stays staged" test "$(git_ -C "$work/dgrants" diff --cached --name-only)" = other.txt
  # X1: a stopped run whose kits are granted since reads so, and leaves pot review's list
  local gsid && gsid=$(d_ runs --json | jq -r '[.runs[] | select(.state == "stopped" and (.stop.fix // "") == "pot grant needs")][0].id')
  ran "dispatch: X1, pot runs after the grant" d_ runs --all
  expect "the stopped run reads granted since" grep -q "^$gsid .*stopped · granted since: start it again" <<<"$out"
  ran "dispatch: X1, pot review" d_ review
  refuse "no longer lists it" grep -q "^$gsid " <<<"$out"
  # X4: pot runs ID shows the command as typed; --json has pot's argv and command.typed
  ran "dispatch: X4, a kit command" d_ needtest -c 'echo hi'
  local xid && xid=$(d_ runs --json | jq -r '[.runs[] | select(.command.name == "needtest")][0].id')
  ran "dispatch: X4, pot runs ID" d_ runs "$xid"
  expect "the command as typed" has "command    needtest -c 'echo hi'"
  expect "--json: argv is pot's" test "$(rec "$xid" '.argv[0] | startswith("/nix/store/")')" = true
  expect "--json: command.typed is the person's" test "$(rec "$xid" '.command.typed | join(" ")')" = "needtest -c echo hi"
  # X2: a detached dispatch started at a terminal is invoker person
  ran "dispatch: X2, --detach at a terminal" tty_ '' "$work/dr/bin/pot" dispatch --detach true
  local tid && tid=$(grep -o '^[0-9a-f]\{4\}' <<<"$out" | head -1)
  expect "is by a person" test "$(rec "$tid" .invoker)" = person
  echo " " >> "$work/dgrants/grants.json"
  refused "dispatch: --commit, the grants file has uncommitted changes" env -u CLAUDECODE -u POT "$work/dr/bin/pot" grant --commit needs </dev/null
  expect "is refused before any prompt" holds "pot grant --commit: $work/dgrants/grants.json has uncommitted changes (git -C $work/dgrants diff HEAD -- grants.json): commit or revert them first, or run pot grant without --commit"
  git_ -C "$work/dgrants" checkout -q -- grants.json
  mv "$work/dgrants/.git" "$work/dgrants/.git.off"
  refused "dispatch: --commit, not in a git repository" env -u CLAUDECODE -u POT "$work/dr/bin/pot" grant --commit </dev/null
  expect "is refused" holds "pot grant --commit: $work/dgrants/grants.json is not in a git repository; run pot grant without --commit"
  mv "$work/dgrants/.git.off" "$work/dgrants/.git"
  refused "dispatch: --commit with --diff" env -u CLAUDECODE -u POT "$work/dr/bin/pot" grant --diff --commit </dev/null
  expect "is usage" holds "pot: grant --commit records grants; --diff only shows them"
  refused "dispatch: S4, --revoke under an agent" env CLAUDECODE=1 "$work/dr/bin/pot" grant --revoke needs </dev/null
  expect "is refused first" holds "pot grant is for people: CLAUDECODE is set"
  expect "and the grant stays" jq -e '.grants["root:needs"]' "$work/dgrants/grants.json"
  refused "dispatch: S4, a named kit under an agent" env CLAUDECODE=1 "$work/dr/bin/pot" grant no-such-kit </dev/null
  expect "is refused before the name is looked up" holds "pot grant is for people: CLAUDECODE is set"
  refused "dispatch: --commit under an agent" env CLAUDECODE=1 "$work/dr/bin/pot" grant --commit </dev/null
  expect "is refused" holds "pot grant is for people: CLAUDECODE is set"
  # R4: --revoke KIT --commit commits the revocation (after the marker check and commit_check)
  ran "dispatch: R4, --revoke --commit" env -u CLAUDECODE -u POT "$work/dr/bin/pot" grant --revoke needs --commit </dev/null
  expect "R4: commits it" grep -qE "^committed $work/dgrants/grants.json \([0-9a-f]{7}\)" <<<"$out"
  expect "R4: its subject" test "$(git_ -C "$work/dgrants" log -1 --format=%s)" = "pot grant --revoke: needs"
  expect "R4: the grant is gone" test "$(jq -r '.grants["root:needs"] // "gone"' "$work/dgrants/grants.json")" = gone
  expect "G3: its body says Revoked on" grep -q "^Revoked on " <<<"$(git_ -C "$work/dgrants" log -1 --format=%b)"
  # R5: pot grant KIT --commit (the synopsis order) reads --commit as a flag
  ran "dispatch: R5, pot grant KIT --commit" tty_ $'y\n' "$work/dr/bin/pot" grant needs --commit
  expect "R5: commits the grant" holds "committed $work/dgrants/grants.json"
  expect "R5: subject" test "$(git_ -C "$work/dgrants" log -1 --format=%s)" = "pot grant: needs"
  # F11: a --commit that fails (a hook refuses) unstages the grants file pot added
  git_ -C "$work/dgrants" rm -q --cached grants.json && git_ -C "$work/dgrants" commit -q -m "untrack grants"
  printf '#!/bin/sh\nexit 1\n' > "$work/dgrants/.git/hooks/pre-commit" && chmod +x "$work/dgrants/.git/hooks/pre-commit"
  env -u CLAUDECODE -u POT "$work/dr/bin/pot" grant --revoke needs </dev/null >/dev/null 2>&1 || true
  out=$(tty_ $'y\n' "$work/dr/bin/pot" grant --commit needs 2>&1 || true)
  expect "F11: the commit failed" holds "pot grant --commit: git commit failed (above); the grants file is written, not committed"
  expect "F11: and the file pot added is no longer staged" test -z "$(git_ -C "$work/dgrants" diff --cached --name-only -- grants.json)"
  rm -f -- "$work/dgrants/.git/hooks/pre-commit"
  git_ -C "$work/dgrants" add grants.json && git_ -C "$work/dgrants" commit -q -m "track grants again"
  ran "dispatch: pot grant, nothing stale" env -u CLAUDECODE -u POT "$work/dr/bin/pot" grant </dev/null
  expect "L2: says there is nothing to grant" holds "pot: nothing to grant"
  expect "L2: and names the kit never granted" holds "pot: also not granted: unused (pot grant NAME)"

  # pot doctor: the backend, the resource scope, the grants file, bindings by path (never contents), instances,
  # this workspace, pot's state, the launcher; exit 1 when a line starts with !
  echo "SECRET-VALUE-$RANDOM" > "$work/dtoken" && chmod 600 "$work/dtoken"
  refused "dispatch: pot doctor" d_ doctor
  local ver && ver=$(sed -n 's/^## v\([0-9.]*\) .*/\1/p' "$src/CHANGELOG.md" | head -1)
  expect "names pot's version, the CHANGELOG's latest" grep -qE "· pot ${ver:-none}\$" <<<"$out"
  expect "the backend" holds "backend    bubblewrap: user namespaces work; proves closure-store"
  expect "the grants file" holds "grants     $work/dgrants/grants.json: parses"
  expect "a binding file, by path" holds "credential api.example: file $work/dtoken (present, 0600)"
  expect "a missing one, marked" holds "! credential api.missing.example: file $work/dtoken-missing is missing"
  expect "X5: with the binding's fix" holds "file $work/dtoken-missing is missing: make-token, saved with mode 600"
  expect "X6: no section wording" holds "no entry for $(hostname -s) in the grants file's machines"
  expect "X6: plurals agree" grep -qE '^state .*: [0-9]+ runs?( |,)' <<<"$out"
  if [ -e "$HOME/.config/pot/launcher" ]; then
    expect "X3: an mkRoot root's launcher fix" holds "point ~/.config/pot/launcher at this root's bin/pot"
    refuse "X3: not home-manager switch" holds "(home-manager switch)"
  fi
  mv "$work/dgrants/grants.json" "$work/dgrants/grants.json.off"
  refused "dispatch: X5, doctor with no grants file" d_ doctor
  expect "says pot grant writes it" holds "grants     $work/dgrants/grants.json: missing: pot grant writes it"
  echo "{" > "$work/dgrants/grants.json"
  refused "dispatch: X5, doctor with a broken grants file" d_ doctor
  expect "names its path once" test "$(grep '^! grants' <<<"$out" | grep -o "$work/dgrants/grants.json" | wc -l)" = 1
  mv "$work/dgrants/grants.json.off" "$work/dgrants/grants.json"
  refuse "never a file's contents" holds "SECRET-VALUE"
  chmod 644 "$work/dtoken"
  refused "dispatch: pot doctor, a key file others can read" d_ doctor
  expect "is marked" holds "credential api.example: file $work/dtoken (present, 0644: others can read it; chmod 600 it)"
  chmod 600 "$work/dtoken"
  expect "this workspace" holds "workspace  dc: repo:fake.example/dc"
  expect "pot's state" grep -qE '^state      .*: [0-9]+ runs' <<<"$out"
  expect "no agent kit, no instances" holds "instances  none (no kit of this root has instances)"
  if systemd-run --user --scope --quiet true 2>/dev/null; then
    expect "the resource scope" holds "resources  user systemd: runs without a terminal get MemoryMax=1G TasksMax=512"
    # L1: where the scope works, resource-limits is not a lack: the backend row says whom it covers
    expect "the backend row: resource-limits for runs without a terminal" holds "; resource-limits: for runs without a terminal"
    refuse "and not among what it lacks" grep -qE '^backend .*lacks .*resource-limits' <<<"$out"
  fi
  # with no systemd-run on PATH (no user systemd): lacking, and marked. The PATH: every program of the test's, but
  # systemd-run (a live probe runs true)
  mkdir -p "$work/dnosd"
  local pd px
  for pd in ${PATH//:/ }; do for px in "$pd"/*; do
    [ -x "$px" ] && [ "${px##*/}" != systemd-run ] && [ ! -e "$work/dnosd/${px##*/}" ] && ln -s "$px" "$work/dnosd/${px##*/}"
  done; done 2>/dev/null
  mkdir -p "$work/dplain" && { [ -d "$work/dplain/.git" ] || { git_ -C "$work/dplain" init -q && git_ -C "$work/dplain" commit -q --allow-empty -m init; }; }
  # shellcheck disable=SC2016 # the inner bash expands them
  ran "dispatch: pot doctor with no user systemd" bash -c 'cd "$1" && env -u POT -u CLAUDECODE PATH="$2" "$3" doctor </dev/null; true' _ \
    "$work/dplain" "$work/dnosd" "$work/dr/bin/pot"
  expect "names resource-limits as lacking" grep -qE '^backend .*lacks .*resource-limits' <<<"$out"
  expect "and marks the resources row" holds "! resources no user systemd"
  # F9: with no true on PATH, doctor names PATH, not user namespaces
  # shellcheck disable=SC2016 # the inner bash expands them
  ran "dispatch: F9, doctor with a PATH that lacks true" bash -c 'cd "$1" && env -u POT -u CLAUDECODE PATH="$2" "$3" doctor </dev/null; true' _ \
    "$work/dplain" "$work/dbin" "$work/dr/bin/pot"
  expect "F9: names PATH" holds "its probe runs true, and PATH has none"
  refuse "F9: not user namespaces" holds "cannot create a user namespace"
  # D2: a run's own probe (no cached result) with the same PATH says the same
  case "$work/dstate/cache" in "$work"/*) rm -f -- "$work/dstate/cache"/probe-*;; esac
  # shellcheck disable=SC2016 # the inner bash expands them
  refused "dispatch: D2, a run with a PATH that lacks true" bash -c 'cd "$1" && env -u POT -u CLAUDECODE PATH="$2" "$3" true </dev/null' _ \
    "$work/dplain" "$work/dbin" "$work/dr/bin/pot"
  expect "D2: names PATH" holds "pot: bubblewrap: its probe runs true, and PATH has none (PATH=$work/dbin); user namespaces were not tested, so no run starts"
  refuse "D2: not user namespaces" holds "can't create a user namespace"
  # shellcheck disable=SC2016
  ran "dispatch: pot doctor inside a run is refused" d_ bash -c 'pot doctor 2>&1; true'
  expect "naming explain and why" holds "pot doctor runs on the host; inside a run, pot explain and pot why"

  # S7: --read of a folder holding pot's state names pot's state, not "launcher mounts"
  refused "dispatch: S7, --read of a folder holding pot's state" d_ --read "$work" true
  expect "names pot's state" holds "which holds pot's own state ($work/dstate)"
  refuse "and no launcher mounts" holds "launcher mounts"
  refused "dispatch: S7, --read inside pot's state" d_ --read "$work/dstate/runs" true
  expect "says it is pot's state" holds "which is pot's own state"

  # refusals and inside a run
  # (a repository that is not a root: a root gets the root refusal first, D5)
  refused "dispatch: --repo in clone mode" d_ dispatch --repo "$work/dplain" true
  expect "is a later release's" holds "pot (dispatch): --repo comes to clone mode in a later release; a dispatch clones this repository only"
  refused "dispatch: --from in place" d_ --from side true
  expect "is usage" holds "pot (work): --from is clone mode: pot dispatch --from REF CMD, or pot --clone --from REF CMD"
  refused "dispatch: --mode in-place on dispatch" d_ dispatch --mode in-place true
  expect "says so" holds "pot (dispatch): --mode in-place: dispatch works in a clone; pot work CMD works here"
  refused "dispatch: --approve" d_ review --approve "$did"
  expect "comes with pot serve" holds "pot review --approve comes with pot serve (phase 3): no run waits for approval in this release"
  refused "dispatch: --root" d_ review --root x
  expect "comes with deployment roots" holds "pot review --root comes with deployment roots (phase 2): pot review reads this root's runs"
  # shellcheck disable=SC2016
  ran "dispatch: pot runs inside a run" d_ bash -c 'pot runs --json; pot dispatch true 2>&1; pot review 2>&1; true'
  expect "shows the run itself" test "$(head -1 <<<"$out" | jq -r '.runs[0].state')" = running
  expect "runs/1: inside a run, the list validates" valid contracts/runs-1.schema.json <(head -1 <<<"$out")
  expect "dispatch inside a run: the host's (decision 19, v2.20.0)" holds "pot (work): pot dispatch is the host's: a run starts no clone inside it; run it on the host"
  expect "review is the host's" holds "pot (work): pot review is the host's: a run's output is reviewed outside it"
  (
    cd "$work/drootrepo"
    refused "dispatch: work in a root repository" d_ true
    expect "names dispatch" holds "pot (work): $work/drootrepo is a root: pot dispatch CMD (its commits come back for review), or pot inspect CMD"
    # a change to a line of a file that sets programs.pot (not a line naming it)
    ran "dispatch: dispatch in a root repository" d_ dispatch bash -c 'sed -i "s/a = 1/a = 2/" pot.nix && git commit -qam root'
    local rr && rr=$(d_ runs --json | jq -r '[.runs[] | select(.workspace[0].path == "'"$work/drootrepo"'" and .state == "review")][0].id')
    expect "its review flags the root change (D13)" test "$(rec "$rr" '.attention | index("root: pot.nix (programs.pot)")')" != null
    # K3: a committed file named :(exclude)* beside the change: run-chosen paths are literal pathspecs
    # shellcheck disable=SC2016
    ran "dispatch: K3, a root change beside a file named :(exclude)*" dl dispatch bash -c 'sed -i "s/a = 1/a = 3/" pot.nix
      echo x > ":(exclude)*" && git --literal-pathspecs add -- pot.nix ":(exclude)*" && git commit -qm k3'
    local k3 && k3=$(sed -n 's/^pot: run \([0-9a-f]\{4\}\) · .*/\1/p' <<<"$out" | tail -1)
    expect "K3: still flags (programs.pot)" test "$(rec "$k3" '.attention | index("root: pot.nix (programs.pot)")')" != null
  )
  # a scion with submodules of its own: a clone holds only the first level, and says so
  prov nst && git_ -C "$work/src/nst" submodule add -q -- "$(url sbd)" deep 2>/dev/null && release nst v1.0.0
  by_hand nst && commit "nst"
  ran "dispatch: a scion with its own submodules" dl dispatch true
  refuse "X7: not on the startup line" holds "scions/nst: its own submodules are not cloned"
  local nid && nid=$(d_ runs --json | jq -r '.runs[0].id')
  ran "dispatch: X7, pot review ID" d_ review "$nid"
  expect "names the left-out submodules" holds "note       scions/nst: its own submodules are not cloned"
  expect "and so does the run's explain" grep -q "scions/nst: its own submodules are not cloned" "$st/$nid/explain.txt"
  # a commit in a scion, on its detached HEAD: back into that scion's own repository
  ran "dispatch: a commit in a scion" d_ dispatch bash -c 'git -C scions/sbd commit -q --allow-empty -m scion-fix'
  local sr && sr=$(d_ runs --json | jq -r '[.runs[] | select(.clone.scionCommits == 1)][0].id')
  expect "is review" test "$(rec "$sr" .state)" = review
  ran "dispatch: --fetch" dl review "$sr" --fetch
  expect "names the submodule" holds "scions/sbd: its commits are in that submodule's refs/remotes/pot-$sr/*"
  expect "into the scion's repository" test "$(git_ -C scions/sbd log -1 --format=%s "refs/remotes/pot-$sr/detached")" = scion-fix
  expect "F1: and no no-op scion branch" test -z "$(git_ -C scions/sbd for-each-ref --format='%(refname)' "refs/remotes/pot-$sr/" | grep -v '/detached$')"
  expect "G3: and no top-level pot-ID/main (the run moved no top-level ref)" test -z "$(git_ for-each-ref "refs/remotes/pot-$sr/")"
  # R7: the exit line counts each moved ref's commits and the scions'
  # shellcheck disable=SC2016
  ran "dispatch: R7, commits on two branches and in a scion" dl dispatch bash -c 'git -C scions/sbd commit -q --allow-empty -m s7
    git commit -q --allow-empty -m t7 && git switch -q -c side7 && git commit -q --allow-empty -m u7'
  expect "R7: names each ref and the scion" grep -qE "^pot: run [0-9a-f]{4} · 1 commit on main, 2 on side7, 1 in scions/sbd: pot review [0-9a-f]{4}, then pot review [0-9a-f]{4} --fetch" <<<"$out"
  local r7 && r7=$(sed -n 's/^pot: run \([0-9a-f]\{4\}\) · .*/\1/p' <<<"$out" | tail -1)
  ran "dispatch: --fetch of a run with two branches" dl review "$r7" --fetch
  expect "its next steps name the run's base branch" holds "  take it: git merge pot-$r7/main"
  # the repo kit label: a commit to a file the repo kit protects (AGENTS.md) is host-run, not a repo-kit change
  # shellcheck disable=SC2016
  ran "dispatch: a commit changing AGENTS.md" d_ dispatch bash -c 'b=$(echo changed | git hash-object -w --stdin)
    git update-index --cacheinfo 100644,"$b",AGENTS.md && git commit -qm agents'
  local ag && ag=$(d_ runs --json | jq -r '.runs[0].id')
  expect "flagged host-run" test "$(rec "$ag" '.attention | index("host-run: AGENTS.md")')" != null
  expect "not labelled repo kit (only the flake's own files are)" test "$(rec "$ag" '[.attention[] | select(startswith("repo kit"))] | length')" = 0
  # v2.19.1: a clone proposes agent configuration (.claude here), while the repo kit's protect (AGENTS.md) and the
  # clone's git hooks stay read-only
  ran "dispatch: a clone proposes agent configuration" d_ dispatch bash -c 'if (echo x >> AGENTS.md) 2>/dev/null; then echo "wrote AGENTS.md"; else echo "AGENTS.md read-only"; fi
    if (echo x > .git/hooks/post-commit) 2>/dev/null; then echo "wrote a hook"; else echo "hooks read-only"; fi
    mkdir -p .claude && echo "{}" > .claude/settings.json && git add .claude && git commit -qm "propose claude settings" && echo committed'
  expect "the repo kit's protect stays read-only" has "AGENTS.md read-only"
  expect "and the clone's hooks" has "hooks read-only"
  expect "agent configuration is writable and commits" has "committed"
  local pc && pc=$(d_ runs --json | jq -r '.runs[0].id')
  expect "flagged as agent configuration" test "$(rec "$pc" '.attention | index("agent config: .claude/settings.json")')" != null
  expect "not as host-run" test "$(rec "$pc" '[.attention[] | select(startswith("host-run"))] | length')" = 0
  expect "with clone.agentConfig" test "$(rec "$pc" '.clone.agentConfig | join(",")')" = ".claude/settings.json"
  # in place, an existing .claude stays protected (protection covers the paths that exist at start)
  mkdir -p .claude && echo "{}" > .claude/settings.json
  ran "dispatch: an in-place run" d_ bash -c 'if (echo x > .claude/new.json) 2>/dev/null; then echo "wrote .claude"; else echo ".claude read-only"; fi'
  expect "may not write .claude" has ".claude read-only"
  case "$PWD/.claude" in "$work"/*) rm -rf -- "$PWD/.claude";; esac
  # X8: when the host signs, the sign line names a branch to rebase on
  ran "dispatch: X8, a commit to sign" d_ dispatch bash -c 'git commit -q --allow-empty -m to-sign'
  local gid && gid=$(d_ runs --json | jq -r '[.runs[] | select(.state == "review" and .clone.commits == 1)][0].id')
  ran "dispatch: X8, --fetch where the host signs" env GIT_CONFIG_VALUE_3=true POT_QUIET=1 env -u CLAUDECODE -u POT "$work/dr/bin/pot" review "$gid" --fetch
  expect "names a branch" grep -qE "^  sign:    git switch -c pot-$gid pot-$gid/main && git rebase --exec 'git commit --amend --no-edit -S' [0-9a-f]{7}" <<<"$out"
  expect "D3: sign first, then take it" test "$(grep -oE '^  (review|sign|take it):' <<<"$out" | tr -d ' ' | paste -sd,)" = "review:,sign:,takeit:"
  expect "D3: take it merges the signed branch" has "  take it: git merge pot-$gid"
  # G4: a root whose stateDir has a space: dispatch and fetch-back work
  ran "dispatch: G4, a root whose stateDir has a space" nix build -o "$work/drs" "path:$work/droot#packages.$sys.spaced"
  ran "dispatch: G4, a dispatch there" env -u POT -u CLAUDECODE POT_QUIET=1 "$work/drs/bin/pot" dispatch bash -c 'git commit -q --allow-empty -m spaced' </dev/null
  local g4 && g4=$(env -u POT POT_QUIET=1 "$work/drs/bin/pot" runs --json | jq -r '.runs[0].id')
  expect "G4: its clone is under the spaced folder" test -d "$work/d state/runs/$g4/clone"
  ran "dispatch: G4, its --fetch" env -u POT -u CLAUDECODE "$work/drs/bin/pot" review "$g4" --fetch </dev/null
  expect "G4: fetch-back works" test "$(git_ log -1 --format=%s "refs/remotes/pot-$g4/main")" = spaced
  # K4: pot's state under inspect's path, holding a killed in-place run's socket and a socket of its own: inspect
  # starts (nothing is masked inside pot's hidden state), and the next pot removes the lost run's sockets
  ran "dispatch: K4, a root whose state is under inspect.home" nix build -o "$work/dri" "path:$work/droot#packages.$sys.inspectstate"
  mkdir -p "$work/dinsp/state"
  python3 -c 'import socket, sys; socket.socket(socket.AF_UNIX).bind(sys.argv[1])' "$work/dinsp/state/other.sock"
  setsid env -u CLAUDECODE -u CODEX_SANDBOX -u POT POT_QUIET=1 "$work/dri/bin/pot" --net allowed.example bash -c 'sleep 300' </dev/null >/dev/null 2>&1 &
  local k4p=$! k4=
  for _ in $(seq 1 100); do k4=$(grep -l "\"pid\":$k4p," "$work/dinsp/state"/runs/*/log.jsonl 2>/dev/null | head -1) && [ -n "$k4" ] && break; sleep 0.2; done
  k4=$(basename "$(dirname "$k4")")
  for _ in $(seq 1 50); do [ -S "$work/dinsp/state/runs/$k4/proxy.sock" ] && break; sleep 0.1; done
  kill -9 -- "-$k4p" 2>/dev/null || true
  wait "$k4p" 2>/dev/null || true
  expect "K4: the killed run left its proxy socket" test -S "$work/dinsp/state/runs/$k4/proxy.sock"
  ran "dispatch: K4, inspect with the state under its path" env -u CLAUDECODE -u CODEX_SANDBOX -u POT -u TMUX -u TMUX_PANE POT_QUIET=1 "$work/dri/bin/pot" inspect bash -c 'echo inspected' </dev/null
  expect "K4: inspect starts" has inspected
  expect "K4: the lost run's socket is gone" test ! -e "$work/dinsp/state/runs/$k4/proxy.sock"
  expect "K4: pot's own socket elsewhere in its state stays" test -S "$work/dinsp/state/other.sock"
  # G4: a .gitmodules name with .. is never a path pot clones from (git's own rule for names)
  local gm=$work/dmods && mkdir -p "$gm" && git_ -C "$gm" init -q && git_ -C "$gm" commit -q --allow-empty -m init
  mkdir -p "$gm/evil" && git_ -C "$gm/evil" init -q && echo planted > "$gm/evil/PLANTED" && git_ -C "$gm/evil" add PLANTED && git_ -C "$gm/evil" commit -q -m evil
  local evc && evc=$(git_ -C "$gm/evil" rev-parse HEAD)
  printf '[submodule "../../evil"]\n\tpath = sub\n\turl = ./evil\n' > "$gm/.gitmodules"
  git_ -C "$gm" add .gitmodules && git_ -C "$gm" update-index --add --cacheinfo "160000,$evc,sub" && git_ -C "$gm" commit -q -m mods
  # shellcheck disable=SC2016 # the inner bash expands them
  ran "dispatch: G4, a dispatch where .gitmodules names a submodule ../../evil" bash -c 'cd "$1" && env -u POT -u CLAUDECODE POT_QUIET=1 "$2" dispatch bash -c "ls -A sub 2>&1; true" </dev/null' _ "$gm" "$work/dr/bin/pot"
  refuse "G4: its submodule is not cloned from the name's path" holds PLANTED
  expect "dispatch: runs/1, every log line and record validates (clone, review, stop, fetch, discard, recovered)" runs_valid "$work/dstate" d_
  expect "dispatch: policy.json/1, every policy.json validates (clones, stops)" policies_valid "$work/dstate"
  expect "dispatch: launch.json/1, every launch.json validates (clones, root repositories)" launches_valid "$work/dstate"
  cd "$work"
}

sec_sibling() {
  # v2.18.0: wrapped kit commands (D4) and the children table (§10); v2.20.0: siblings, not nesting (decisions
  # 11-22, docs/reference/pot-siblings.md). nkp: a provider whose kit 1.2 declares commands with no requests (a
  # check, a hook command), and one that needs reach, in its kit.nix; a fixture flake hands it to kitsFor as a
  # producer would, and gives tools with the wrappers over the programs (as tuteur's mkFlake does).
  prov nkp
  cat > "$work/src/nkp/kit.nix" <<'EOF'
{ pkgs, ... }:
let
  chk = pkgs.writeShellScriptBin "nk-chk" ''
    case "''${1:-}" in
      fail) echo "nk-chk: failing"; exit 7 ;;
      sleep) trap 'echo "nk-chk: INT"; exit 130' INT; echo "nk-chk: sleeping"; ${pkgs.coreutils}/bin/sleep "$2" & wait; exit 0 ;;
      fd0) if [ -p /dev/stdin ]; then echo "nk-chk fd0: a pipe"; else echo "nk-chk fd0: not a pipe"; fi
        if ${pkgs.coreutils}/bin/ls /proc/self/fd/0/ >/dev/null 2>&1; then echo "nk-chk fd0: a directory it can list"; fi; exit 0 ;;
      cat) exec ${pkgs.coreutils}/bin/cat ;;
      inject) (echo injected > /proc/self/fd/0) 2>/dev/null; echo "nk-chk: tried"; exit 0 ;;
      procs) n=0; for p in /proc/[0-9]*; do case "$(${pkgs.coreutils}/bin/tr '\0' ' ' < "$p/cmdline" 2>/dev/null)" in *"sleep 3100"*) n=$((n+1)) ;; esac; done
        echo "nk-chk procs=$n"; exit 0 ;;
    esac
    echo "nk-chk POT=$POT parent=$(${pkgs.jq}/bin/jq -r '.mounts.parent // "-"' /run/pot/policy.json 2>/dev/null) args=$*"
    if touch nk-chk-wrote 2>/dev/null; then echo "nk-chk: wrote"; else echo "nk-chk: read-only"; fi
    if ${pkgs.util-linux}/bin/unshare -U true 2>/dev/null; then echo "nk-chk: userns"; else echo "nk-chk: no userns"; fi
    echo "nk-chk proxy=''${HTTPS_PROXY:-none} tok=''${AGTOK:-none} stand=''${STAND:-none}"
    # v2.19.1: a project .pi, sealed in the child too (its mounts come from its parent's view)
    if [ -d .pi ]; then
      if mkdir .pi/settings.json.lock 2>/dev/null; then echo "nk-chk: .pi lock ok"; rmdir .pi/settings.json.lock; else echo "nk-chk: .pi lock refused"; fi
      if (echo x > .pi/settings.json) 2>/dev/null; then echo "nk-chk: .pi wrote"; else echo "nk-chk: .pi read-only"; fi
      echo left > .pi/child-left 2>/dev/null || true
    fi
  '';
  hook = pkgs.writeShellScriptBin "nk-hook" ''
    if [ -e hookfail ]; then echo "nk-hook: failing"; exit 3; fi
    echo "nk-hook POT=$POT index=''${GIT_INDEX_FILE:-unset} staged=$(git diff --cached --name-only | tr '\n' ' ')"
  '';
  wide = pkgs.writeShellScriptBin "nk-wide" "echo nk-wide ran";
  net = pkgs.writeShellScriptBin "nk-net" ''
    for p in "$@"; do curl -s --noproxy "" --proxy "$HTTP_PROXY" -o /dev/null -w "nk-net $p %{http_code}\n" "http://nothing.invalid:$p/"; done
    cat /run/pot/denials
  '';
in
{
  tools = { inherit chk hook wide net; };
  kit = {
    version = "1.2";
    commands.nk-chk = { program = "${chk}/bin/nk-chk"; access = "read"; description = "the check"; };
    commands.nk-hook = { program = "${hook}/bin/nk-hook"; access = "read"; hook = "pre-commit"; };
    commands.nk-wide = { program = "${wide}/bin/nk-wide"; requests.env = [ "WIDE_TOK" ]; };
    commands.nk-net = { program = "${net}/bin/nk-net"; access = "read"; requests.network = [ "nothing.invalid:1" ]; };
    recipes.check = { run = "nk-chk --all"; description = "Run the check"; };
    recipes.wide = "nk-wide";
  };
}
EOF
  release nkp v1.0.0
  local c=$work/nc && mkdir -p "$c" && git_ -C "$c" init -q && cd "$c"
  cat > flake.nix <<EOF
{
  # the submodule's kit.nix is in the flake's source (a git flake leaves submodules out without it; pot builds path:)
  inputs.self.submodules = true;
  inputs.pot.url = "path:$pot";
  outputs = { pot, ... }: let
    pkgs = pot.inputs.nixpkgs.legacyPackages.$sys;
    nkp = import ./scions/nkp/kit.nix { inherit pkgs; };
    tools = builtins.attrValues nkp.tools;
    potKits = pot.lib.pot.kitsFor {
      inherit pkgs tools;
      scions.nkp = { inherit (nkp) kit; file = "scions/nkp/kit.nix"; };
      files = [ "**/kit.nix" "scions" ];
      recipes.producer = { run = "nk-wide producer"; description = "The producer's own"; };
    };
  in {
    packages.$sys = {
      pot-kits = potKits;
      tools = pkgs.buildEnv { name = "tools"; paths = tools ++ [ (pkgs.lib.hiPrio potKits.wrappers) ];
        passthru.unwrapped = pkgs.buildEnv { name = "tools"; paths = tools; }; };
    };
  };
}
EOF
  echo data > notes.md && mkdir -p .githooks
  # shellcheck disable=SC2016 # the hook's own variables
  printf '#!/bin/sh\nexec "${POT_TOOLS:-%s/nct/bin}/nk-hook" "$@"\n' "$work" > .githooks/pre-commit && chmod +x .githooks/pre-commit
  printf 'import? "scions/kits.just"\n' > justfile
  by_hand nkp && commit init && quiet nix flake lock && commit lock

  # the bundle and the tools (pot-lib/1)
  ran "sibling: pot-kits builds" nix build -o "$work/ncpk" .#pot-kits
  expect "bin/nk-chk is the wrapper" test "$(readlink "$work/ncpk/bin/nk-chk")" = "$(readlink -f "$work/ncpk")/wrappers/bin/nk-chk"
  expect "each command has its manifest" test "$(jq -c . "$work/ncpk/commands/nk-hook.json")" = '{"command":"nk-hook","kit":"scions.nkp","version":1}'
  ran "sibling: kits.just" cat "$work/ncpk/kits.just"
  expect "has the kit's recipe, its words quoted" holds 'exec "{{tools}}/bin/nk-chk" '"'"'--all'"'"' "$@"'
  expect "and the producer's own" holds 'exec "{{tools}}/bin/nk-wide" '"'"'producer'"'"' "$@"'
  expect "with its description" has "# Run the check"
  ran "sibling: the tools" nix build -o "$work/nct" .#tools
  expect "hold the wrapper" grep -q 'run --command nk-chk --manifest' "$work/nct/bin/nk-chk"
  ran "sibling: tools.passthru.unwrapped" nix build -o "$work/nctu" .#tools.passthru.unwrapped
  expect "holds the program itself" test "$(readlink -f "$work/nctu/bin/nk-chk")" = "$(jq -r '.scions.nkp.commands["nk-chk"].program' "$work/ncpk/kit.json")"
  # kits.just, as a producer writes it beside its submodules (protected in a run, as the bundle's files say)
  cp "$work/ncpk/kits.just" scions/kits.just && chmod 644 scions/kits.just && commit kitsjust
  # F1: every kit recipe is checked inside the kitsJust evaluation (a producer writes it from nix eval, no build)
  local exj="let g = builtins.getFlake \"path:$pot\"; pkgs = g.inputs.nixpkgs.legacyPackages.$sys; in"
  # shellcheck disable=SC2016 # Nix text, not shell
  for probe in 'lint = "lint $(touch /tmp/pwned)"' 'esc = "../../../run/current-system/sw/bin/sh -c id"' '"x\n    echo injected\n\nzz" = "lint"' 'pipe = "lint a|sh"'; do
    refused "sibling: kitsJust refuses the recipe $probe" nix eval --impure --raw --expr "$exj (g.lib.pot.kitsFor { inherit pkgs; scions.s = { file = \"s/kit.nix\"; kit = { commands.lint.program = \"/bin/true\"; recipes.$probe; }; }; }).kitsJust"
    expect "with pot's check" holds "pot: s/kit.nix: recipes."
  done

  # the root: an agent kit with reach (network, a variable); bubblewrap from the test's nixpkgs
  local bw && bw=$(nix build --no-link --print-out-paths "$np#bubblewrap")/bin/bwrap
  if ! "$bw" --unshare-all --unshare-user --disable-userns --ro-bind / / true 2>/dev/null; then
    echo "skip nest: runs (bubblewrap can't create an unprivileged user namespace here)"
    return 0
  fi
  local r=$work/nroot && mkdir -p "$r"
  cat > "$r/flake.nix" <<EOF
{
  inputs.pot.url = "path:$pot";
  inputs.nixpkgs.follows = "pot/nixpkgs";
  outputs = { pot, nixpkgs, ... }: let pkgs = nixpkgs.legacyPackages.$sys;
    common = {
      inherit pkgs;
      name = "nest";
      grantsFile = "$work/ngrants/grants.json";
      stateDir = "$work/nstate";
      rootRepos = [ "$work/nrr" ];
      kits.agent.commands.agent = { program = "\${pkgs.bashInteractive}/bin/bash";
        requests = { network = [ "nothing.invalid:1" "nothing.invalid:2" ]; env = [ "AGTOK" ]; }; };
      # standing reach (decision 15: a kit command never gets it)
      kits.stand.requests = { network = [ "nothing.invalid:3" ]; env = [ "STAND" ]; };
      contexts.work.include = [ "stand" ];
    }; in {
    packages.$sys.pot = pot.lib.mkRoot common;
    # the same state, clones kept 0 days (H5: a child in review is never pruned)
    packages.$sys.prune = pot.lib.mkRoot (common // { clones = { keepDays = 0; maxSize = "20G"; }; });
  };
}
EOF
  (cd "$r" && quiet nix flake lock)
  ran "sibling: the root builds" nix build -o "$work/nr" "path:$r#packages.$sys.pot"
  expect "its pot takes pot run (share/pot/run-1)" test -e "$(readlink -f "$work/nr")/share/pot/run-1"
  # nb_ ARGS: the test root's pot; ntty_ ANSWERS CMD: on a terminal
  # shellcheck disable=SC2317
  nb_() { env -u CLAUDECODE -u CODEX_SANDBOX -u POT AGTOK=secret-agtok STAND=standing WIDE_TOK=w "$work/nr/bin/pot" "$@"; }
  # shellcheck disable=SC2317
  ntty_() { local a=$1; shift; (sleep 2; printf '%s' "$a") | script -qec "env -u CLAUDECODE -u CODEX_SANDBOX -u POT AGTOK=secret-agtok STAND=standing $*" /dev/null; }
  # wrapped_ CMD...: CMD on the host with the test root as the launcher (~/.config/pot/launcher, bound over the real
  # one in a mount namespace of its own, so no test reaches the person's own pot)
  mkdir -p "$work/nlaunch" && ln -sfn "$(readlink -f "$work/nr")/bin/pot" "$work/nlaunch/launcher"
  # shellcheck disable=SC2317
  wrapped_() { env -u CLAUDECODE -u CODEX_SANDBOX -u POT AGTOK=secret-agtok "$bw" --dev-bind / / \
    --bind "$work/nlaunch" "$HOME/.config/pot" -- "$@"; }
  ran "sibling: grant the agent" ntty_ $'y\n' "$work/nr/bin/pot" grant agent
  ran "sibling: grant nkp" ntty_ $'y\n' "$work/nr/bin/pot" grant nkp
  ran "sibling: grant stand" ntty_ $'y\n' "$work/nr/bin/pot" grant stand

  # the children table (explain)
  ran "sibling: explain agent" nb_ explain agent
  expect "lists the siblings" holds "nk-chk nk-hook nk-net (nkp): sibling · agent (agent): exec"
  expect "refuses what needs reach the run lacks" holds "  nk-wide: refused (needs WIDE_TOK)"
  refuse "no dispatch rows (decision 19)" holds "dispatch agent"
  ran "sibling: explain --json agent" nb_ explain --json agent
  nb_ explain --json agent > "$work/nexp.json" 2>/dev/null
  expect "validates against policy.json/1 (exec and sibling rows)" valid contracts/policy-1.schema.json "$work/nexp.json"
  ran "the hook command's row" jq -r '.children | to_entries[] | select(.value.command == "nk-hook") | .value.hook' "$work/nexp.json"
  expect "names its hook" has "pre-commit"
  expect "no row has a sub-table (each sibling's is composed at its start)" test "$(jq '[.children[] | select(has("children"))] | length' "$work/nexp.json")" = 0
  ran "sibling: explain true (no command kit)" nb_ explain true
  expect "only the workspace's commands" holds "children   nk-chk nk-hook (nkp): sibling"
  expect "and a sibling with network, from a run with none, is refused" holds "  nk-net: refused (needs nothing.invalid:1)"

  # inside a run: a wrapped command is a sibling the host starts, under its own policy alone
  ran "sibling: a wrapped command inside a run" nb_ agent -c 'nk-chk a b'
  expect "runs, in its own run" holds "nk-chk POT=work parent=- args=a b"
  expect "read-only, as its access says" has "nk-chk: read-only"
  expect "with no user namespaces" has "nk-chk: no userns"
  expect "without the caller's network, variable or token, or the context's standing reach (decision 15)" \
    has "nk-chk proxy=none tok=none stand=none"
  ran "sibling: the caller's processes" nb_ agent -c 'sleep 3100 & nk-chk procs; kill %1'
  expect "are not the sibling's (its own pid namespace)" has "nk-chk procs=0"
  refused "sibling: its exit status comes back" nb_ agent -c 'nk-chk fail'
  expect "the sibling's output" has "nk-chk: failing"
  local st; nb_ agent -c 'nk-chk fail' >/dev/null 2>&1 && st=0 || st=$?
  expect "and its exit code, 7" test "$st" = 7
  ran "sibling: its stdin and stdout" nb_ agent -c 'echo piped | nk-chk cat; nk-chk cat < notes.md; nk-chk cat < notes.md > out.txt; cat out.txt; rm out.txt'
  expect "a pipe passes" has "piped"
  expect "a file is piped by the client" holds "data"
  # the review (v2.20.0): a caller's pipe never crosses as it is, so a sibling cannot write into its caller's input
  # shellcheck disable=SC2016 # the run's own variables
  ran "sibling: a caller's input pipe" nb_ agent -c '(sleep 4; echo later) | { nk-chk inject; read -r x; echo "caller-read:$x"; }'
  expect "the sibling ran" has "nk-chk: tried"
  expect "and the caller reads only its own input" has "caller-read:later"
  ran "sibling: a directory as stdin" nb_ agent -c 'nk-chk fd0 < .'
  expect "gets /dev/null for it: nothing of the caller's view" has "nk-chk fd0: not a pipe"
  refuse "and lists nothing through it" holds "a directory it can list"
  # the parent: its own reach, user namespaces off (decision 13: every run)
  local ul && ul=$(grep -o '/nix/store/[^ ]*util-linux[^/ ]*/bin/unshare' "$(jq -r '.scions.nkp.commands["nk-chk"].program' "$work/ncpk/kit.json")" | head -1)
  ran "sibling: the caller itself" nb_ agent -c "echo \"proxy=\${HTTPS_PROXY:-none} tok=\$AGTOK stand=\$STAND\"; $ul -U true 2>/dev/null && echo userns || echo no-userns"
  expect "has its reach" holds "proxy=http://127.0.0.1:3128 tok=secret-agtok stand=standing"
  expect "and no user namespaces either (--disable-userns in every run)" has "no-userns"
  refused "sibling: a command that needs reach the caller lacks" nb_ agent -c 'nk-wide'
  expect "is refused at start" holds "pot (work): nk-wide needs WIDE_TOK, which this run (agent) lacks: run it on the host (just wide)"
  # a sibling with network: its own proxy, from the host, its allow list its own (the caller reaches :1 and :2)
  ran "sibling: a sibling with network" nb_ agent -c 'nk-net 1 2'
  expect "reaches its own host (the port is closed: 502)" has "nk-net 1 502"
  expect "and is refused, by its own proxy, a host only the caller reaches" has "nk-net 2 403"
  expect "its denial names its kit's requests.network" holds "  permanent: requests.network in nkp (scions/nkp/kit.nix)"
  ran "sibling: its record on the host" bash -c "grep -h '\"via\":\"sibling\"' '$work'/nstate/runs/*/log.jsonl | head -1"
  # shellcheck disable=SC2016 # the run's own variables
  expect "is the host's (by: host), naming the run that asked" bash -c 'jq -e "select(.by == \"host\" and (.caller | test(\"^[0-9a-f]{4}$\")) and .invoker == \"run\" and .depth == 1)" <<<"$1" >/dev/null' _ "$out"
  refuse "sibling: no children folder in any run" bash -c "ls -d '$work'/nstate/runs/*/children '$work'/nstate/runs/*/table 2>/dev/null | grep -q ."
  mkdir -p "$work/nplain" && git_ -C "$work/nplain" init -q
  ran "sibling: a run with nothing to start" env -C "$work/nplain" POT_QUIET=1 "$work/nr/bin/pot" --package util-linux bash -c "unshare -U true 2>/dev/null && echo userns || echo no-userns"
  expect "keeps --disable-userns" has "no-userns"
  # decision 16 (a wrapper with no row: 126, never run as the caller) is test_children's NoRow: a wrapper from another
  # workspace's tools is not in a run's closure here

  # the broker refuses what a forged client sends (the review list: floods, forged requests, fd passing)
  local py && py=$(grep -o '/nix/store/[^ ]*/bin/python3' "$(readlink -f "$work/nr")/bin/pot" | head -1)
  cat > "$work/nforge.py" <<'EOF'
import json, os, socket, sys
def ask(head, fds, raw=None):
    s = socket.socket(socket.AF_UNIX); s.connect("/run/pot/children.sock")
    socket.send_fds(s, [raw if raw is not None else json.dumps(head).encode() + b"\n"], fds)
    print(s.makefile().readline().strip())
key = [k for k, r in json.load(open("/run/pot/policy.json"))["children"].items() if r.get("command") == "nk-chk"][0]
d = os.open(".", os.O_RDONLY)
ask({"v": 1, "key": key, "args": ["fd0"], "cwd": os.getcwd(), "hook": {}}, [d, 1, 2])
ask({"v": 1, "key": "root:agent:agent", "args": [], "cwd": "/", "hook": {}}, [0, 1, 2])
ask({"v": 1, "key": "nope", "args": [], "cwd": "/", "hook": {}}, [0, 1, 2])
ask(None, [0, 1, 2], raw=b"{not json\n")
# the review's F2: malformed requests with fds, past select()'s ceiling (1024) had their fds leaked
for _ in range(400):
    ask(None, [0, 1, 2], raw=b"{not json\n")
print("leaks-sent")
EOF
  cp "$work/nforge.py" nforge.py
  ran "sibling: forged requests" nb_ agent -c "$py -I nforge.py"
  expect "a directory fd is refused" holds '"refused": "pot (work): nk-chk: fds 0-2 go to a sibling only as a terminal, a pipe, a socket or /dev/null"'
  expect "an exec row is no sibling" holds '"refused": "pot (work): agent: not a sibling row in this run'"'"'s children table: run it on the host", "exit": 126'
  expect "an unknown key neither" holds '"refused": "pot (work): nope: not a sibling row'
  expect "and a malformed request is refused" holds "the run's broker refused the request"
  expect "400 more were sent" has "leaks-sent"
  local st2; nb_ agent -c 'nk-chk fail' >/dev/null 2>&1 && st2=0 || st2=$?
  expect "sibling: after them, a sibling still runs and returns its code (no fd leaked)" test "$st2" = 7
  # the review's F1: a sibling never makes the host evaluate what the run wrote (a nested flake, here)
  # shellcheck disable=SC2016 # the run's own variables
  ran "sibling: from a nested repository the run made, with a flake" nb_ agent -c 'mkdir -p nest && git -C nest init -q &&
    printf "{ outputs = _: builtins.trace \"SIBLING-EVAL-MARKER\" { }; }\n" > nest/flake.nix && git -C nest add flake.nix &&
    cd nest && nk-chk n; cd ..; rm -rf nest'
  expect "the sibling runs (from the caller's repository top)" holds "nk-chk POT=work parent=- args=n"
  refuse "and the host evaluated nothing the run wrote" holds "SIBLING-EVAL-MARKER"
  refuse "nor built a repo kit at the sibling's start" holds "building the repo kit"
  rm -f nforge.py

  # cancellation (decision 14): ^C, the client killed (the agent's tool timeout), the caller's run ending
  # shellcheck disable=SC2016 # the run's own $?
  out=$( (sleep 6; printf '\003'; sleep 4) | script -qec "env -u CLAUDECODE -u CODEX_SANDBOX -u POT AGTOK=x $work/nr/bin/pot agent -c 'nk-chk sleep 3101; echo client-exit=\$?'" /dev/null 2>&1 | tr -d '\r')
  pass "sibling: ^C at the terminal"
  expect "reaches the sibling" holds "nk-chk: INT"
  expect "whose status comes back" holds "client-exit=130"
  ran "sibling: the client killed" nb_ agent -c 'nk-chk sleep 3102 & sleep 3; kill -9 $!; sleep 4; echo done'
  expect "stops the sibling" test "$(pgrep -fc 'sleep 3102' || true)" = 0
  ran "sibling: the client's SIGTERM (a tool timeout)" nb_ agent -c 'nk-chk sleep 3103 & sleep 3; kill $!; wait $!; echo "client-exit=$?"; sleep 4'
  expect "the client exits 143" has "client-exit=143"
  expect "and the sibling is stopped" test "$(pgrep -fc 'sleep 3103' || true)" = 0
  ran "sibling: the caller's run ends" nb_ agent -c 'nk-chk sleep 3104 & sleep 3; echo leaving'
  sleep 4
  expect "stops the sibling with it" test "$(pgrep -fc 'sleep 3104' || true)" = 0
  # v2.20.1: a host pot killed by SIGTERM (outside a sibling) skips its backend's cleanup; the next start clears the
  # run's children.sock and its tree's slots, once its backend has ended (not while it still runs)
  nb_ agent -c 'nk-chk k >/dev/null; sleep 3106; true' >/dev/null 2>&1 &
  local kbg=$! krd="" kid="" kpid="" d
  for _ in $(seq 60); do
    for d in "$work"/nstate/runs/*/; do
      [ -S "$d/children.sock" ] && [ -d "$work/nstate/trees/$(basename "$d")" ] && krd=${d%/} && break 2
    done
    sleep 0.5
  done
  kid=$(basename "${krd:-none}") && kpid=$(jq -r 'select(.ev == "start") | .pid' "$krd/log.jsonl" 2>/dev/null || true)
  expect "sibling: a run with a sibling has its socket and its tree's slots" test -n "$kpid"
  kill -TERM "$kpid" && { wait "$kbg" 2>/dev/null || true; }
  ran "sibling: its host pot killed by SIGTERM, another run starts while its command runs" nb_ agent -c true
  expect "which leaves the live run's socket" test -S "$krd/children.sock"
  pkill -f 'sleep 3106' || true
  for _ in $(seq 30); do [ -e "$krd/status" ] && break; sleep 0.5; done
  expect "its backend ends" test -e "$krd/status"
  ran "sibling: another run starts after it" nb_ agent -c true
  expect "and clears its children.sock" test ! -e "$krd/children.sock"
  expect "and its tree's slots" test ! -e "$work/nstate/trees/$kid"
  # the tree-wide cap (decision 18): 16 at once; the 17th is refused
  # shellcheck disable=SC2016 # the run's own variables
  ran "sibling: 17 at once" nb_ agent -c 'for i in $(seq 17); do nk-chk sleep 3105 > /dev/null 2>"cap.$i" & done; sleep 12; cat cap.*; rm -f cap.*; kill $(jobs -p) 2>/dev/null; sleep 4; echo done'
  expect "one is refused" holds "pot (work): nk-chk: 16 commands already run in this run's tree (pot's cap): wait for one to end, or run it on the host"
  expect "and none is left" test "$(pgrep -fc 'sleep 3105' || true)" = 0

  # ruling 2: inside a v2.18.0 run, a wrapper with no launcher refuses (126); a run an older pot started execs
  # (the wrapper alone, with a stand-in /run/pot/policy.json: one a v2.18.0 pot wrote names the launcher)
  mkdir -p "$work/nrun" && echo '{"mounts": {"launcher": "~/.config/pot/launcher"}}' > "$work/nrun/new.json" && echo '{"mounts": {}}' > "$work/nrun/old.json"
  "$bw" --dev-bind / / --tmpfs /run --ro-bind "$work/nrun/new.json" /run/pot/policy.json --tmpfs "$HOME/.config/pot" \
    -- "$work/nct/bin/nk-chk" >"$work/nrun/out" 2>&1 && st=0 || st=$?
  out=$(cat "$work/nrun/out")
  expect "sibling: in a v2.18.0 run with no launcher, exit 126" test "$st" = 126
  expect "naming the command" holds "pot: nk-chk: this run's launcher (/run/pot/launcher) is missing, so its children table can't be reached: run nk-chk on the host"
  refuse "and nothing ran unwrapped" holds "nk-chk POT"
  ran "sibling: in a run an older pot started" "$bw" --dev-bind / / --tmpfs /run --ro-bind "$work/nrun/old.json" /run/pot/policy.json \
    --tmpfs "$HOME/.config/pot" -- "$work/nct/bin/nk-chk" o
  expect "execs under that run's policy, as before" holds "args=o"
  # conformance 2: an older pot's run that shows a pointer (home-manager's, in inspect) still runs the program
  mkdir -p "$work/nold/bin" "$work/noldl" && printf '#!/bin/sh\necho old-pot "$@"\n' > "$work/nold/bin/pot" && chmod +x "$work/nold/bin/pot"
  ln -sfn "$work/nold/bin/pot" "$work/noldl/launcher"
  ran "sibling: in an older pot's run, with a pointer in the home" "$bw" --dev-bind / / --tmpfs /run --ro-bind "$work/nrun/old.json" /run/pot/policy.json \
    --bind "$work/noldl" "$HOME/.config/pot" -- "$work/nct/bin/nk-chk" o2
  expect "runs the program, never the pointer" holds "args=o2"
  refuse "(the old pointer never runs)" holds "old-pot"

  # hook commands: git commit inside a run runs the hook command as a sibling, on the right index (-a, PATH); a
  # failing hook aborts the commit with its own code. (Commits on the host go through wrapped_ only.)
  git_ config core.hooksPath .githooks
  echo more >> notes.md
  ran "sibling: git commit -a inside a run" nb_ agent -c 'git commit -q -a -m a'
  expect "runs the hook command" holds "nk-hook POT=work index=$c/.git/index.lock staged=notes.md"
  echo again >> notes.md && echo other > other.md && git_ add other.md
  ran "sibling: git commit PATH inside a run" nb_ agent -c 'git commit -q -m b notes.md'
  # shellcheck disable=SC2016 # bash -c's own arguments
  expect "on git's temporary index" bash -c 'grep -qE "$1" <<<"$2"' _ "index=$c/\.git/next-index-[0-9]+\.lock staged=notes\.md" "$out"
  git_ -c core.hooksPath=/dev/null commit -q -m c
  local head0 && head0=$(git_ rev-parse HEAD)
  touch hookfail && echo fail >> notes.md
  refused "sibling: a failing hook inside a run" nb_ agent -c 'git commit -q -a -m nope; echo "commit-exit=$?"; exit 1'
  expect "runs" has "nk-hook: failing"
  expect "and aborts the commit" has "commit-exit=1"
  expect "which never happened" test "$(git_ rev-parse HEAD)" = "$head0"
  rm -f hookfail && git_ checkout -q notes.md

  # on the host: a wrapper runs pot run, quiet, with the command's own policy and no standing reach (decision 15)
  ran "sibling: a wrapper on the host" wrapped_ "$work/nct/bin/nk-chk" x
  expect "runs in a run of its own" holds "nk-chk POT=work parent=- args=x"
  expect "read-only" has "nk-chk: read-only"
  expect "without the agent's variable or the standing reach" has "nk-chk proxy=none tok=none stand=none"
  refuse "quiet: no startup line" holds "pot: work ·"
  ran "sibling: logged with via: wrapper" bash -c "grep -l '\"via\":\"wrapper\"' '$work'/nstate/runs/*/log.jsonl"
  ran "sibling: pot runs hides it (exit 0)" nb_ runs
  refuse "unless --all" holds "nk-chk x"
  ran "sibling: pot runs --all" nb_ runs --all
  expect "lists it" holds "nk-chk x"
  echo host >> notes.md
  ran "sibling: git commit -a on the host" wrapped_ git commit -q -a -m d
  expect "runs the hook command through pot" holds "nk-hook POT=work index=$c/.git/index.lock staged=notes.md"
  refused "sibling: a wrapper outside its workspace" bash -c "cd '$work' && '$bw' --dev-bind / / --bind '$work/nlaunch' \"\$HOME/.config/pot\" -- '$work/nct/bin/nk-chk'"
  expect "says where to run it" holds "pot: nk-chk runs in its repository (cd there first)"
  refused "sibling: pot run with a bad manifest" nb_ run --command nk-chk --manifest /tmp/x.json
  expect "is usage" holds "pot run: --manifest /tmp/x.json is not a wrapper's manifest (<bundle>/commands/nk-chk.json)"
  refused "sibling: pot run --sibling by hand, with no request" nb_ run --sibling 0 </dev/null
  expect "is not a broker's request" holds "pot: pot run --sibling: not a broker's request"

  # the wrapper's fallbacks: no launcher, an older root's pot, Darwin
  ran "sibling: a wrapper with no launcher" env -u POT "$bw" --dev-bind / / --tmpfs "$HOME/.config/pot" -- "$work/nct/bin/nk-chk" y
  expect "runs the program, saying so" holds "pot: nk-chk runs without pot (no root here: ~/.config/pot/launcher is missing)"
  expect "unwrapped" holds "nk-chk POT= parent= args=y"
  ran "sibling: under an older root's pot" env -u POT "$bw" --dev-bind / / --bind "$work/noldl" "$HOME/.config/pot" -- "$work/nct/bin/nk-chk" z
  expect "names the fix" holds "pot: nk-chk runs without pot (your root's pot predates wrapped commands: take pot 0.1.0 or later in your root (home-manager switch))"
  refuse "never runs the old pot run" holds "old-pot"
  refuse "sibling: the wrapper has no Darwin case of its own" grep -q uname "$work/ncpk/wrappers/bin/nk-chk"

  # decision 19: no nested dispatch or --detach; dispatch stays the host's, and its clone's hook runs as a sibling
  refused "sibling: pot dispatch inside a run" nb_ agent -c 'pot dispatch agent -c true'
  expect "is the host's" holds "pot (work): pot dispatch is the host's: a run starts no clone inside it; run it on the host"
  refused "sibling: pot --clone inside a run" nb_ agent -c 'pot --clone agent -c true'
  expect "too" holds "pot dispatch is the host's"
  refused "sibling: pot dispatch --detach inside a run" nb_ agent -c 'pot dispatch --detach agent -c true'
  expect "too" holds "pot dispatch is the host's"
  refused "sibling: pot review inside a run" nb_ agent -c 'pot review'
  expect "is the host's" holds "pot (work): pot review is the host's: a run's output is reviewed outside it"
  ran "sibling: pot runs inside a run" nb_ agent -c 'pot runs --json'
  expect "lists the run alone" test "$(grep '^{' <<<"$out" | jq '.runs | length')" = 1
  expect "runs/1: and validates" valid contracts/runs-1.schema.json <(grep '^{' <<<"$out")
  git_ -c core.hooksPath=/dev/null commit -q -a -m before-dispatch --allow-empty
  ran "sibling: a host dispatch whose commit runs the hook" nb_ dispatch agent -c 'echo x > n.md; git add n.md; git commit -q -m in-clone; echo committed'
  expect "the hook command ran, in the clone (as a sibling)" holds "nk-hook POT=dispatch index=$c/.git/index staged=n.md"
  expect "and the commit is there" has "committed"
  local did && did=$(sed -n 's/^pot: run \([0-9a-f]*\) · 1 commit on main.*/\1/p' <<<"$out")
  expect "it comes back as the dispatch's commit (the sibling staged nothing of its own)" test -n "$did"
  ran "sibling: the dispatch's record" nb_ runs "$did" --json
  expect "one commit" test "$(jq -r '.clone.commits' <<<"$out")" = 1

  # H8: a read-only kit command runs in a root repository; a write run there is refused
  git_ clone -q --recurse-submodules "$c" "$work/nrr"
  ran "sibling: a read-only wrapped command in a root repository" bash -c "cd '$work/nrr' && '$bw' --dev-bind / / --bind '$work/nlaunch' \"\$HOME/.config/pot\" -- '$work/nct/bin/nk-chk' rr"
  expect "runs" holds "args=rr"
  refused "sibling: a write run in the root repository" bash -c "cd '$work/nrr' && env -u POT AGTOK=x '$work/nr/bin/pot' agent -c true"
  expect "is refused" holds "is a root"
  # F2: the access is the kit's that runs: a read wrapper from another bundle, where this root repository's kit says write
  # shellcheck disable=SC2016 # Nix text, not shell
  sed -i 's/commands.nk-chk = { program = "${chk}\/bin\/nk-chk"; access = "read";/commands.nk-chk = { program = "${chk}\/bin\/nk-chk"; access = "write";/' "$work/src/nkp/kit.nix"
  release nkp v1.1.0
  git_ -C "$work/nrr/scions/nkp" fetch -q --tags origin && git_ -C "$work/nrr/scions/nkp" checkout -q --detach v1.1.0
  git_ -C "$work/nrr" -c core.hooksPath=/dev/null commit -q -a -m "nkp v1.1.0"
  refused "sibling: F2, a read wrapper whose kit here writes, in a root repository" bash -c "cd '$work/nrr' && '$bw' --dev-bind / / --bind '$work/nlaunch' \"\$HOME/.config/pot\" -- '$work/nct/bin/nk-chk' rr"
  expect "is refused, as work refuses a root" holds "is a root"
  refuse "nothing ran" holds "args=rr"
  expect "sibling: runs/1, every log line and record validates (siblings, wrappers, hooks)" runs_valid "$work/nstate" nb_
  expect "sibling: policy.json/1, every policy.json validates (siblings, wrappers, hooks)" policies_valid "$work/nstate"
  expect "sibling: launch.json/1, every launch.json validates (siblings, hooks)" launches_valid "$work/nstate"
}

# pi (kits/pi.nix): a root with the pi kit, its program a stub that reports what it sees. The run's pi
# home is its state, with pi's configuration fixed (read-only) in it and its folders read-only; it reaches
# its providers' hosts alone; its key comes from its binding. The host's pi folder is one the host lacks, so
# nothing of a real ~/.pi is read.
sec_pi() {
  # the kit's evaluation errors (fix round 1: F1's trailing dot, F2, F3, I3), each by its message
  cat > "$work/pi-err.nix" <<'EOF'
{ pot, case }:
let
  g = builtins.getFlake pot;
  pkgs = g.inputs.nixpkgs.legacyPackages.${builtins.currentSystem};
  pi = a: g.kits.pi ({ inherit pkgs; } // a);
  one = p: pi { providers.p = p; };
  cases = {
    dot = one { baseUrl = "http://localhost.:8000/v1"; };
    cleartext = one { baseUrl = "http://x.example.test/v1"; apiKey = "\${K}"; };
    cleartext-endpoint = pi { endpoints = [ { url = "http://x.example.test"; key = "K"; } ]; };
    header-command = one { baseUrl = "https://x.example.test"; headers.Authorization = "!pass key"; };
    override-command = one { baseUrl = "https://x.example.test"; modelOverrides.m.headers.X = "!cmd"; };
    endpoint-key = pi { endpoints = [ { url = "https://x.example.test"; key = "not a name"; } ]; };
    port = one { baseUrl = "https://x.example.test:0443/v1"; };
    ipv6 = one { baseUrl = "https://[2001:db8::1]/v1"; };
  };
in cases.${case}.commands.pi.requests
EOF
  # shellcheck disable=SC2317 # run through refused
  pierr() { nix eval --impure --json --expr "import $work/pi-err.nix { pot = \"path:$pot\"; case = \"$1\"; }"; }
  refused "pi: a trailing dot at the loopback" pierr dot
  expect "is the loopback" holds 'pot.kits.pi: provider p: http://localhost.:8000/v1 is the host'"'"'s loopback, which a run cannot reach (host-loopback is reserved for a later release): leave it out'
  refused "pi: F2, a variable's key on http" pierr cleartext
  expect "names the cleartext" holds 'pot.kits.pi: provider p: http://x.example.test/v1 would carry K in cleartext: give an https url, or a literal key'
  refused "pi: F2, an endpoint's key on http" pierr cleartext-endpoint
  expect "the same" holds 'pot.kits.pi: endpoints #1: http://x.example.test would carry K in cleartext: give an https url, or a literal key'
  refused "pi: F3, a header that runs a command" pierr header-command
  # shellcheck disable=SC2016 # pi's own interpolation, as text
  expect "names the header" holds 'pot.kits.pi: provider p: headers.Authorization runs a command ("!…") in the run: give "${NAME}" and bind NAME'
  refused "pi: F3, in modelOverrides too" pierr override-command
  expect "names where" holds 'pot.kits.pi: provider p, modelOverrides.m: headers.X runs a command ("!…") in the run'
  refused "pi: I3, an endpoint's key that is no variable name" pierr endpoint-key
  expect "says key, not apiKey" holds 'pot.kits.pi: endpoints #1: key "not a name": give the variable'"'"'s name ("NAME")'
  refused "pi: I3, a port with a leading zero" pierr port
  expect "names the provider" holds 'pot.kits.pi: provider p: https://x.example.test:0443/v1: the port is 1 to 65535, with no leading zero'
  refused "pi: I3, an IPv6 literal" pierr ipv6
  expect "names the provider" holds 'pot.kits.pi: provider p: https://[2001:db8::1]/v1: an IPv6 address is not a host pot'"'"'s network requests name: give a host name'

  local r=$work/piroot && mkdir -p "$r"
  cat > "$r/flake.nix" <<EOF
{
  inputs.pot.url = "path:$pot";
  inputs.nixpkgs.follows = "pot/nixpkgs";
  outputs = { pot, nixpkgs, ... }: {
    packages = nixpkgs.lib.genAttrs [ "$sys" ] (s: let pkgs = nixpkgs.legacyPackages.\${s}; in {
      pot = pot.lib.mkRoot {
        inherit pkgs;
        name = "pi-test";
        grantsFile = "$work/pigrants/grants.json";
        stateDir = "$work/pistate";
        kits.pi = pot.kits.pi {
          inherit pkgs;
          binary = pkgs.writeShellScriptBin "pi" (builtins.readFile ./stub-pi.sh);
          agentDir = ".pot-test-no-such-pi";
          files.a = { target = ".pot-test-no-such-pi/AGENTS.md"; source = pkgs.writeText "agents" "root instructions"; };
          settings.defaultProvider = "gw";
          providers.gw = { baseUrl = "https://api.pi-test.example/v1"; apiKey = "\\\${PITEST_KEY}"; models = [ { id = "m"; } ]; };
          providers.lan = { baseUrl = "http://lan.pi-test.example:11434/v1"; apiKey = "literal"; models = [ { id = "q"; } ]; };
        };
        bindings."status.relay"."*" = { workmux = true; };
        bindings.credential."api.pi-test.example kit=pi" = { via = "env"; from.file = "$work/pikey"; };
        contexts.readonly = { extends = "work"; access = "read"; };
        # v2.19.1: hidden paths under the workspace (untracked, tracked, inside a sealed folder, a hidden .pi)
        hidden = [ "$work/piws/secret.txt" "$work/piws/tracked.txt" "$work/piws/sfile/.pi/secret.json" "$work/piws/hp/.pi" ];
      };
    });
  };
}
EOF
  # the stub: what pi would see, one line each (curl -p: CONNECT for http too, as pi's proxy agent does)
  cat > "$r/stub-pi.sh" <<'STUB'
d=$PI_CODING_AGENT_DIR
echo "env PI_OFFLINE=$PI_OFFLINE PI_TELEMETRY=$PI_TELEMETRY PI_SKIP_VERSION_CHECK=$PI_SKIP_VERSION_CHECK dir=${d#"$HOME"}"
echo "auth $(cat "$d/auth.json") mcp-auth $(cat "$d/mcp-auth.json") mcp $(tr -d ' \n' < "$d/mcp.json")"
echo "agents $(cat "$d/AGENTS.md") / override $(cat "$d/AGENTS.override.md")"
echo "models $(jq -c '[(.providers | keys[]), .providers.gw.apiKey]' "$d/models.json")"
for f in settings.json models.json auth.json mcp.json AGENTS.override.md SYSTEM.md extensions/x.ts skills/x npm/x; do
  # a folder the run could create would be writable: only a bound, read-only one refuses
  if (mkdir -p "$(dirname "$d/$f")" && echo planted > "$d/$f") 2>/dev/null; then echo "wrote $f"; else echo "read-only $f"; fi
done
mkdir -p "$d/sessions/--w--" && echo '{"type":"message","message":{"role":"user","content":"hi"}}' > "$d/sessions/--w--/t_1.jsonl" && echo "wrote sessions"
[ "${PITEST_KEY:-}" = pi-key-value ] && echo "key set"
for u in https://api.pi-test.example/ http://lan.pi-test.example:11434/ https://denied.pi-test.example/; do
  echo "reach $u $(curl -s -p -o /dev/null -w '%{http_connect}' --max-time 20 "$u")"
done
STUB
  (cd "$r" && quiet nix flake lock)
  ran "pi: a root with the pi kit builds" nix build -o "$work/pir" "path:$r#packages.$sys.pot"
  local bw && bw=$(nix build --no-link --print-out-paths "$np#bubblewrap")/bin/bwrap
  if [ "$(uname)" = Darwin ] || ! "$bw" --unshare-all --unshare-user --disable-userns --ro-bind / / true 2>/dev/null; then
    echo "skip pi: runs (bubblewrap's, which need an unprivileged user namespace)"
    return 0
  fi
  local c=$work/piws && mkdir -p "$c" && git_ -C "$c" init -q && cd "$c"
  # v2.19.1: a project's .pi (protected, **/.pi) that names a skills folder, and a probe that reads its settings as
  # pi 1.5.2 does (proper-lockfile: mkdir settings.json.lock, read, rmdir), then tries to change the folder
  mkdir -p .pi/skills/x && echo '{"skills":["./skills"]}' > .pi/settings.json && echo "skill x" > .pi/skills/x/SKILL.md
  cat > probe-pi.sh <<'PROBE'
p=$PWD/.pi
if err=$(mkdir "$p/settings.json.lock" 2>&1); then
  echo "project settings $(tr -d ' \n' < "$p/settings.json")"; rmdir "$p/settings.json.lock"
else echo "WARNING the lock: $err"; fi
[ "${1:-}" = read ] && exit 0
for f in settings.json skills/x/SKILL.md; do (echo planted > "$p/$f") 2>/dev/null && echo "wrote $f" || echo "read-only $f"; done
mkdir "$p/skills/y" 2>/dev/null && echo "made skills/y" || echo "read-only skills/"
echo replaced > "$TMPDIR/s.json" 2>/dev/null || echo replaced > /tmp/s.json
mv -f "${TMPDIR:-/tmp}/s.json" "$p/settings.json" 2>/dev/null && echo "replaced settings.json" || echo "kept settings.json"
mv "$p/settings.json" "$p/moved.json" 2>/dev/null && echo "renamed settings.json" || echo "not renamed"
rm -f "$p/settings.json" 2>/dev/null; rm -rf "$p/skills" 2>/dev/null
[ "$(cat "$p/settings.json" 2>/dev/null)" = '{"skills":["./skills"]}' ] && echo "settings.json as it was" || echo "settings.json CHANGED"
[ -e "$p/skills/x/SKILL.md" ] && echo "skills kept" || echo "skills DELETED"
if [ -e "$p/extensions/x.ts" ]; then echo "an earlier run's extensions/x.ts is here"; else echo "no earlier extensions/x.ts"; fi
mkdir -p "$p/extensions" && echo "x" > "$p/extensions/x.ts" && echo "made extensions/x.ts"
PROBE
  commit init
  # shellcheck disable=SC2317 # run through ran and expect
  pi_() { env -u CLAUDECODE -u CODEX_SANDBOX -u POT -u PI_CODING_AGENT POT_QUIET=1 "$work/pir/bin/pot" "$@"; }
  # shellcheck disable=SC2317
  pitty_() { local a=$1; shift; (sleep 2; printf '%s' "$a") | script -qec "env -u CLAUDECODE -u CODEX_SANDBOX -u POT -u PI_CODING_AGENT $*" /dev/null; }
  ran "pi: explain" pi_ explain pi
  expect "the kit, with no instance" holds "run        pi -> root:pi"
  expect "its providers' hosts" holds "api.pi-test.example:443"
  expect "the http one on its port" holds "lan.pi-test.example:11434"
  expect "the key by its binding" holds "PITEST_KEY"
  printf 'PITEST_KEY=pi-key-value\n' > "$work/pikey" && chmod 600 "$work/pikey"
  ran "pi: pot grant pi" pitty_ $'y\ny\n' "$work/pir/bin/pot" grant pi
  ran "pi: a run" env -u CLAUDECODE -u CODEX_SANDBOX -u POT -u PI_CODING_AGENT "$work/pir/bin/pot" pi
  expect "the startup line names no instance" grep -qE "^pot: work · piws · pot explain" <<<"$out"
  expect "offline at start, no telemetry, its own home" has "env PI_OFFLINE=1 PI_TELEMETRY=0 PI_SKIP_VERSION_CHECK=1 dir=/.pi/agent"
  expect "the credential files are empty (a /login does not persist)" has 'auth {} mcp-auth {} mcp {"mcpServers":{}}'
  expect "the root's instructions, also as the override" has "agents root instructions / override root instructions"
  # shellcheck disable=SC2016 # pi's own interpolation, kept as text
  expect "models.json is the root's, its key a reference" has 'models ["gw","lan","${PITEST_KEY}"]'
  local f
  for f in settings.json models.json auth.json mcp.json AGENTS.override.md SYSTEM.md extensions/x.ts skills/x npm/x; do
    expect "pi: $f is read-only" has "read-only $f"
  done
  expect "the sessions are the run's state" has "wrote sessions"
  expect "the key comes from its binding" has "key set"
  expect "a provider's host is reached (allowed; no such host: 502)" has "reach https://api.pi-test.example/ 502"
  expect "and the http one" has "reach http://lan.pi-test.example:11434/ 502"
  expect "another host is refused" has "reach https://denied.pi-test.example/ 403"
  expect "its denial's first fix is the root's pi kit (I2)" holds "  permanent: a provider or endpoint in kits.pi (pot.kits.pi; rebuild the root)"
  expect "the session is in the run's state" test -n "$(find "$work/pistate" -name t_1.jsonl)"
  expect "the exit line resumes it with --continue" holds "pot: resume this session with: pot pi --continue"

  # v2.19.1: the project's .pi is sealed: pi's read lock works (a new entry, in a folder of the run's own), and every
  # entry that was there stays read-only; a new entry lasts for the run alone
  ran "pi: .pi sealed, in place" pi_ bash probe-pi.sh
  expect "pi reads the project's settings, with no warning" has 'project settings {"skills":["./skills"]}'
  refuse "no lock warning" holds "WARNING the lock"
  expect "settings.json stays read-only" has "read-only settings.json"
  expect "a subfolder stays read-only throughout" has "read-only skills/x/SKILL.md"
  expect "and takes no new entry" has "read-only skills/"
  expect "it cannot be replaced" has "kept settings.json"
  expect "or renamed" has "not renamed"
  expect "or deleted" has "settings.json as it was"
  expect "nor the skills" has "skills kept"
  expect "a new top-level entry is made" has "made extensions/x.ts"
  expect "pi: the host's .pi is unchanged" test "$(cat .pi/settings.json)" = '{"skills":["./skills"]}'
  refuse "and has no new entry" test -e .pi/extensions
  refuse "and no lock left" test -e .pi/settings.json.lock
  ran "pi: the next run" pi_ bash probe-pi.sh
  expect "does not see the last run's new entry" has "no earlier extensions/x.ts"
  ran "pi: .pi sealed, in a read-only context" pi_ readonly bash probe-pi.sh read
  expect "pi reads the settings there too" has 'project settings {"skills":["./skills"]}'
  ran "pi: .pi sealed, in inspect" pi_ inspect bash probe-pi.sh read
  expect "and in inspect" has 'project settings {"skills":["./skills"]}'
  # v2.19.1: a clone proposes agent configuration: there .pi is writable (no seal: the clone is the scratch copy), and
  # what it commits is flagged by the host's own look, in review and on the fetch line
  ran "pi: a clone proposes .pi" pi_ dispatch bash -c 'bash probe-pi.sh read; echo "{\"skills\":[\"./skills\"],\"proposed\":1}" > .pi/settings.json && echo "wrote .pi in the clone"; if (echo x > .git/hooks/pre-commit) 2>/dev/null; then echo "wrote hooks"; else echo "hooks read-only"; fi; git add -A && git commit -q -m "propose pi settings" && echo committed'
  expect "the clone reads the settings" has 'project settings {"skills":["./skills"]}'
  expect "and writes them" has "wrote .pi in the clone"
  expect "the clone's .git hooks stay read-only" has "hooks read-only"
  expect "and commits" has "committed"
  local prid && prid=$(pi_ runs --json | jq -r '[.runs[] | select(.mode == "clone" and .state == "review")][0].id')
  expect "pi: the proposal's run id" test -n "$prid"
  expect "the host's .pi is unchanged until merge" test "$(cat .pi/settings.json)" = '{"skills":["./skills"]}'
  ran "pi: pot review RUN" pi_ review "$prid"
  expect "flags the agent configuration" has "changes agent configuration: .pi/settings.json (loads in agent sessions once merged)"
  ran "pi: pot review RUN --json" pi_ review "$prid" --json
  expect "as clone.agentConfig" test "$(jq -c .clone.agentConfig <<<"$out")" = '[".pi/settings.json"]'
  expect "and an attention item" test "$(jq -r '.attention[]' <<<"$out" | grep -c '^agent config: .pi/settings.json$')" = 1
  ran "pi: pot review RUN --fetch" pi_ review "$prid" --fetch
  expect "the fetch line flags it too" has "  changes agent configuration: .pi/settings.json (loads in agent sessions once merged)"
  expect "the proposal came back" test "$(git_ show "refs/remotes/pot-$prid/main:.pi/settings.json")" = '{"skills":["./skills"],"proposed":1}'
  expect "the host's .pi is still unchanged" test "$(cat .pi/settings.json)" = '{"skills":["./skills"]}'
  ran "pi: explain dispatch names what a clone proposes" pi_ explain dispatch
  expect "on the limits row" holds "agent config: **/.claude **/.mcp.json **/.pi **/.codex **/.vscode (a clone's proposal)"
  refuse "and no seal there" holds "sealed: "
  # each .pi under the top is sealed on its own (a scion's, a subfolder's)
  mkdir -p sub/.pi && echo s > sub/.pi/x
  ran "pi: a subfolder's .pi, sealed too" pi_ bash -c 'cd sub && mkdir .pi/l && rmdir .pi/l && echo "sub: new entry ok"; if (echo y > .pi/x) 2>/dev/null; then echo "sub: wrote x"; else echo "sub: x read-only"; fi'
  expect "takes a new entry" has "sub: new entry ok"
  expect "its entries read-only" has "sub: x read-only"
  case "$PWD/sub" in "$work"/*) rm -rf -- "$PWD/sub";; esac
  ran "pi: explain names the seal" pi_ explain
  expect "on the protect row" holds "sealed: **/.pi (pot)"

  # fix round (v2.19.1) 5: what a run leaves in a sealed folder is reported at exit, then discarded: a file, a
  # folder's file, a link (never followed), and no lock folder
  # shellcheck disable=SC2016
  ran "pi: a run leaves files in the sealed .pi" env -u CLAUDECODE -u CODEX_SANDBOX -u POT -u PI_CODING_AGENT "$work/pir/bin/pot" bash -c 'echo x > .pi/new.txt; ln -s /etc/passwd .pi/evil; mkdir .pi/settings.json.lock; mkdir -p .pi/extensions && echo y > .pi/extensions/x.ts; echo left'
  expect "the exit line lists them, the link as a link" holdsj "pot: the run created .pi/evil (a link), .pi/extensions/x.ts (2 B), .pi/new.txt (2 B) in a sealed folder; it was discarded (to keep agent config: pot dispatch …, then pot review)"
  refuse "no lock folder" holds "settings.json.lock"
  refuse "the host's .pi has none of them" test -e .pi/new.txt -o -L .pi/evil -o -e .pi/extensions
  local srun && srun=$(sed -n 's/^pot: run \([0-9a-f]\{4\}\)$/\1/p' <<<"$out")
  expect "the log records them" test "$(jq -c 'select(.ev == "after") | [.sealed_left[].type]' "$work/pistate/runs/$srun/log.jsonl")" = '["link","file","file"]'
  refuse "and the seal's base is gone" test -e "$work/pistate/runs/$srun/sealed"

  # how pot shows the workspace's path (under ~, as ~/…)
  local cshown=$c && case "$c" in "$HOME"/*) cshown="~${c#"$HOME"}";; esac
  # fix round 3: a sealed folder over the cap (256 entries) stays plainly read-only, with a start note
  mkdir -p x/.pi && (cd x/.pi && seq 1 300 | xargs touch)
  ran "pi: a sealed folder over the cap" env -u CLAUDECODE -u CODEX_SANDBOX -u POT -u PI_CODING_AGENT "$work/pir/bin/pot" bash -c 'if mkdir x/.pi/l 2>/dev/null; then echo "x: new entry"; else echo "x: read-only"; fi'
  expect "starts, with a note" has "pot: $cshown/x/.pi has 300 entries, more than a sealed folder takes (256): it is read-only in this run, so pi cannot take its settings lock there"
  expect "and the folder is plainly read-only" has "x: read-only"
  case "$PWD/x" in "$work"/*) rm -rf -- "$PWD/x";; esac

  # fix round 4 and the hidden finding: a hidden path under the workspace is masked in place and with read access
  # (untracked, inside a sealed folder, a hidden .pi whose seal is skipped); a tracked one gets a start note; a clone
  # is not masked; inspect is as before
  echo SECRET1 > secret.txt && mkdir -p sfile/.pi hp/.pi && echo SECRET2 > sfile/.pi/secret.json && echo ok > sfile/.pi/other
  echo HIDDENPI > hp/.pi/key && echo TRACKED > tracked.txt && git_ add tracked.txt && git_ commit -q -m tracked
  local ctx
  for ctx in work readonly inspect; do
    # shellcheck disable=SC2016 # the run's own shell expands them
    ran "pi: hidden paths under the workspace ($ctx)" env -u CLAUDECODE -u CODEX_SANDBOX -u POT -u PI_CODING_AGENT "$work/pir/bin/pot" "$ctx" bash -c 'echo "s=$(cat secret.txt) s2=$(cat sfile/.pi/secret.json) o=$(cat sfile/.pi/other) hp=$(ls -A hp/.pi | wc -l) t=$(cat tracked.txt)"'
    expect "masked, the sealed folder's other entry readable" has "s= s2= o=ok hp=0 t="
    refuse "no secret" holds "SECRET"
  done
  ran "pi: a hidden path that git tracks" env -u CLAUDECODE -u CODEX_SANDBOX -u POT -u PI_CODING_AGENT "$work/pir/bin/pot" bash -c 'true'
  expect "gets a start note" has "pot: $cshown/tracked.txt is hidden here, but git tracks it: the run sees it empty, and git's history still holds it; don't commit it from this run"
  ran "pi: a commit that holds it empty" env -u CLAUDECODE -u CODEX_SANDBOX -u POT -u PI_CODING_AGENT "$work/pir/bin/pot" bash -c 'git commit -qam blanked && echo blanked'
  expect "is named on the exit line" holdsj "pot: WARNING: the run committed tracked.txt empty: hidden here, the run saw it empty; don't push or merge that commit as it is"
  local brun && brun=$(sed -n 's/^pot: run \([0-9a-f]\{4\}\)$/\1/p' <<<"$out")
  expect "and in the run's attention" test "$(pi_ runs "$brun" --json | jq -r '.attention | index("hidden committed empty: tracked.txt")')" != null
  git_ reset -q --hard HEAD~1
  expect "the host's file is as it was" test "$(cat tracked.txt)" = TRACKED
  # shellcheck disable=SC2016 # the run's own shell expands them
  ran "pi: a clone is not masked" pi_ dispatch bash -c 'echo "t=$(cat tracked.txt)"'
  expect "it holds what git holds" has "t=TRACKED"
  case "$PWD/sfile" in "$work"/*) rm -rf -- "$PWD/sfile" "$PWD/hp" "$PWD/secret.txt";; esac

  # fix round 1: a proposal path that is a link stops a clone as it stops in place; a link inside a real one names
  # agent configuration where it points
  local lw=$work/pilink && mkdir -p "$lw/agents/claude" && git_ -C "$lw" init -q && cd "$lw"
  echo '{}' > agents/claude/settings.json && ln -s agents/claude .claude && git_ add -A && commit linked
  refused "pi: a clone whose .claude is a link" pi_ dispatch true
  expect "stops, as in place" holds "pot (dispatch): .claude is a symbolic link, so the sandbox can't make it read-only"
  git_ rm -q .claude && mkdir -p .pi tools/ext && echo a > tools/ext/a.ts && ln -s ../tools/ext .pi/ext && git_ add -A && commit inner
  ran "pi: a clone writes through a link inside .pi" pi_ dispatch bash -c 'echo b > tools/ext/b.ts && git add -A && git commit -qm through && echo committed'
  local lid && lid=$(pi_ runs --json | jq -r '[.runs[] | select(.state == "review")][0].id')
  expect "flagged where the link points" test "$(pi_ review "$lid" --json | jq -c .clone.agentConfig)" = '["tools/ext/b.ts"]'

  # fix round 2: a scion's agent configuration and host-run files, changed in a clone, are flagged
  local up=$work/piup sw=$work/pisub
  mkdir -p "$up/.claude" && git_ -C "$up" init -q && echo '{}' > "$up/.claude/settings.json" && echo 1 > "$up/f" \
    && git_ -C "$up" add -A && git_ -C "$up" commit -qm up
  mkdir -p "$sw" && git_ -C "$sw" init -q && cd "$sw" && git_ submodule add -q "$up" kit && commit withkit
  ran "pi: a clone changes a scion's agent configuration" pi_ dispatch bash -c 'cd kit && echo "{\"hooks\":1}" > .claude/settings.json && echo "* text" > .gitattributes && git add -A && git commit -qm "kit change" && echo committed'
  local kid2 && kid2=$(pi_ runs --json | jq -r '[.runs[] | select(.state == "review")][0].id')
  ran "pi: its review" pi_ review "$kid2" --json
  expect "flags the scion's agent configuration" test "$(jq -c .clone.agentConfig <<<"$out")" = '["kit/.claude/settings.json"]'
  expect "and its host-run file" test "$(jq -r '.attention | index("host-run: kit/.gitattributes")' <<<"$out")" != null
  cd "$c"
  refused "pi: pot grant under pi's marker" env -u CLAUDECODE -u CODEX_SANDBOX PI_CODING_AGENT=true "$work/pir/bin/pot" grant
  expect "is refused (the kit's agent.markers)" holds "pot grant is for people: PI_CODING_AGENT is set"
  expect "pi: runs/1, every log line and record validates" runs_valid "$work/pistate" pi_
  expect "pi: policy.json/1, every policy.json validates" policies_valid "$work/pistate"
  expect "pi: launch.json/1, every launch.json validates" launches_valid "$work/pistate"
  cd "$work"
}


# rollback (a release check, not in the default run): pot 0.1.0 writes nothing that graft v2.21.0's pot can't read.
# Two roots with the same arguments, one from this pot and one from POT_ROLLBACK_FROM (default
# github:danielnaab/graft/v2.21.0), share one grants file (grants/1) and one state folder (runs/1, the cache):
# this pot grants and runs, then the old one runs on the same state without asking again, lists and fetches this
# pot's runs, and leaves the grants as they were; then this pot reads what the old one wrote. Both pots' state is
# in the work directory, never the person's own.
sec_rollback() {
  local from=${POT_ROLLBACK_FROM:-github:danielnaab/graft/v2.21.0}
  local bw && bw=$(nix build --no-link --print-out-paths "$np#bubblewrap")/bin/bwrap
  if ! "$bw" --unshare-all --unshare-user --disable-userns --ro-bind / / true 2>/dev/null; then
    echo "skip rollback (bubblewrap can't create an unprivileged user namespace here)"
    return 0
  fi
  # the workspace: a repo kit and a submodule kit with reach, in a bundle this pot's kitsFor builds
  prov rbk && release rbk v1.0.0
  local c=$work/rbc && mkdir -p "$c" && git_ -C "$c" init -q && cd "$c"
  cat > flake.nix <<EOF
{
  inputs.pot.url = "path:$pot";
  outputs = { pot, ... }: let pkgs = pot.inputs.nixpkgs.legacyPackages.$sys;
    tool = pkgs.writeShellScriptBin "rbk-tool" "echo rbk-tool ran"; in {
    packages.$sys.pot-kits = pot.lib.pot.kitsFor {
      inherit pkgs;
      self = { kit = { protect = [ "AGENTS.md" ]; tools = [ pkgs.hello ]; }; file = "flake.nix (fixture)"; };
      scions.rbk = { file = "kits/rbk.nix"; kit = { requests.network = [ "api.rbk.example" ]; commands.rbk-tool.program = "\${tool}/bin/rbk-tool"; }; };
      tools = [ tool ];
      files = [ "scions" ];
    };
  };
}
EOF
  echo "# agents" > AGENTS.md && echo data > notes.md
  by_hand rbk && commit init && quiet nix flake lock && commit lock
  # the roots: one set of arguments, two pots
  local r=$work/rbroot && mkdir -p "$r"
  cat > "$r/flake.nix" <<EOF
{
  inputs.pot.url = "path:$pot";
  inputs.old.url = "$from";
  inputs.old.inputs.nixpkgs.follows = "pot/nixpkgs";
  outputs = { pot, old, ... }: let pkgs = pot.inputs.nixpkgs.legacyPackages.$sys;
    common = {
      inherit pkgs;
      name = "rollback";
      grantsFile = "$work/rbgrants/grants.json";
      stateDir = "$work/rbstate";
      kits.daniel.requests.network = [ "nothing.invalid:1" ];
      contexts.work.include = [ "daniel" ];
      kits.agent.commands.agent = { program = "\${pkgs.bashInteractive}/bin/bash"; requests.network = [ "nothing.invalid:2" ]; };
    }; in {
    packages.$sys.new = pot.lib.mkRoot common;
    packages.$sys.old = old.lib.mkRoot common;
  };
}
EOF
  (cd "$r" && quiet nix flake lock)
  ran "rollback: this pot's root builds" nix build -o "$work/rbnew" "path:$r#packages.$sys.new"
  ran "rollback: and the old pot's ($from)" nix build -o "$work/rbold" "path:$r#packages.$sys.old"
  # shellcheck disable=SC2317 # run through ran, expect and refuse
  new_() { env -u CLAUDECODE -u CODEX_SANDBOX -u POT POT_QUIET=1 "$work/rbnew/bin/pot" "$@"; }
  # shellcheck disable=SC2317
  old_() { env -u CLAUDECODE -u CODEX_SANDBOX -u POT POT_QUIET=1 "$work/rbold/bin/pot" "$@"; }
  # shellcheck disable=SC2317
  rtty_() { local a=$1; shift; (sleep 2; printf '%s' "$a") | script -qec "env -u CLAUDECODE -u CODEX_SANDBOX -u POT $*" /dev/null; }

  # this pot: grants and runs (in place, and a dispatch left in review)
  ran "rollback: this pot grants daniel" rtty_ $'y\n' "$work/rbnew/bin/pot" grant daniel
  ran "rollback: and agent" rtty_ $'y\n' "$work/rbnew/bin/pot" grant agent
  ran "rollback: and the submodule kit" rtty_ $'y\n' "$work/rbnew/bin/pot" grant rbk
  ran "rollback: this pot runs agent" new_ agent -c 'echo new-ran'
  expect "it ran" has new-ran
  ran "rollback: this pot runs the submodule kit's command" new_ rbk-tool
  expect "it ran" has "rbk-tool ran"
  ran "rollback: this pot dispatches" new_ dispatch bash -c 'echo n > n.txt && git add n.txt && git commit -qm from-new'
  local nid && nid=$(new_ runs --json | jq -r '[.runs[] | select(.state == "review")][0].id')
  expect "a run in review" test -n "$nid" -a "$nid" != null
  cp "$work/rbgrants/grants.json" "$work/rbgrants.before"

  # the old pot, on the same state: no grant asked again, this pot's runs listed and fetched, the grants unchanged
  ran "rollback: the old pot explains" old_ explain agent
  refuse "nothing is ungranted" holds "not granted"
  ran "rollback: the old pot runs agent, with no terminal (a grant it lacked would stop it: 125)" old_ agent -c 'echo old-ran'
  expect "it ran" has old-ran
  ran "rollback: the old pot runs the submodule kit's command" old_ rbk-tool
  expect "it ran" has "rbk-tool ran"
  ran "rollback: the old pot lists this pot's runs" old_ runs --json --all
  expect "the dispatch, in review" test "$(jq -r --arg id "$nid" '.runs[] | select(.id == $id) | .state' <<<"$out")" = review
  expect "valid runs/1" valid contracts/runs-1.schema.json
  ran "rollback: the old pot fetches this pot's dispatch" old_ review "$nid" --fetch
  expect "its commit is here" test "$(git_ log -1 --format=%s "refs/remotes/pot-$nid/main")" = from-new
  expect "rollback: the old pot left the grants as they were" cmp -s "$work/rbgrants.before" "$work/rbgrants/grants.json"
  ran "rollback: the old pot dispatches" old_ dispatch bash -c 'echo o > o.txt && git add o.txt && git commit -qm from-old'
  local oid && oid=$(old_ runs --json | jq -r '[.runs[] | select(.state == "review")][0].id')

  # this pot again, on what the old one wrote
  ran "rollback: this pot runs again, with no terminal" new_ agent -c 'echo new-again'
  expect "it ran" has new-again
  ran "rollback: this pot lists the old pot's runs" new_ runs --json --all
  expect "its dispatch, in review" test "$(jq -r --arg id "$oid" '.runs[] | select(.id == $id) | .state' <<<"$out")" = review
  ran "rollback: this pot fetches it" new_ review "$oid" --fetch
  expect "its commit is here" test "$(git_ log -1 --format=%s "refs/remotes/pot-$oid/main")" = from-old
  expect "rollback: the grants are still as this pot wrote them" cmp -s "$work/rbgrants.before" "$work/rbgrants/grants.json"
  expect "rollback: every log line and record validates" runs_valid "$work/rbstate" new_
  expect "rollback: every policy.json validates (this pot's and the old pot's)" policies_valid "$work/rbstate"
  cd "$work"
}

sections=(contracts kits pot pi dispatch sibling)
[ $# -eq 0 ] || sections=("$@")
for s in "${sections[@]}"; do
  echo "== $s"
  "sec_$s"
done
echo "all passed"
