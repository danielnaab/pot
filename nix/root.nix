# pot.lib.mkRoot: a root as data (design §1, §13.1), and its `pot`.
#
#   pot.lib.mkRoot {
#     pkgs;                         # the root's nixpkgs (--package builds from it alone)
#     grantsFile ? "~/.config/pot/grants.json";   # host-only, hidden from every run (D7)
#     stateDir ? "~/.local/state/pot";             # runs, logs, caches, state: hidden
#     contexts ? { };               # added to, or changing, host inspect work dispatch (§5)
#     defaultContext ? "work";
#     kits ? { };                   # root kits, by name ("claude-code@personal", "daniel")
#     defaultInstance ? { };        # per kit, the instance chosen with no @NAME, --instance or caller match
#                                   # ({ claude-code = "personal"; })
#     agentMarkers ? [ ];           # variables that mean an agent is the caller, beyond the floor
#                                   # (CODEX_SANDBOX, CLAUDECODE) and the kits' own agent.markers
#     hidden ? [ ];                 # beyond pot's defaults (Appendix B)
#     sealedProtect ? [ "**/.pi" ]; # protected folders that are sealed under bubblewrap (v2.19.1): their entries
#                                   # read-only, and a new entry the run's alone (pi's settings lock)
#     tools ? [ ];                  # packages on every run's PATH, after the standard set
#     bindings ? { };               # bindings.<capability>."<selector>" (§7.2; carried out from milestone 2)
#     rootRepos ? [ ];              # the root's source repositories: work refuses them (D13, D23)
#     repoKits ? { };               # repo kits for repositories with no pot-kits of their own, by origin pattern
#     inspect ? { };                # inspect.home: what inspect reads in ~ (replaces [ "~/src" ])
#     caBundle ? null;              # a CA bundle (a path), at /etc/ssl/certs/ca-certificates.crt in runs
#     limits ? { };                 # the resource scope of runs without a terminal: { memoryMax = "50%"; tasksMax = 2048; }
#     clones ? { };                 # dispatch clones kept: { keepDays = 14; maxSize = "20G"; } (never unfetched commits)
#     container ? null;             # Linux: the container backend too (pot phase 2): { engine = "podman" | "docker";
#                                   # program (an absolute path, or a package with bin/ENGINE; podman's default
#                                   # pkgs.podman, docker's required: never the engine on PATH);
#                                   # image ? docker's empty image (localhost/pot-empty:1) }
#     defaultBackend ? null;        # the backend of a context whose backend is "auto": bubblewrap (Seatbelt on
#                                   # Darwin) unless "container"
#     releaseSigners ? null;        # 0.6.0: an allowed_signers file (a path): a changed source that only moves pins
#                                   # to release tags these signers signed keeps its grant (never the repository's)
#   }
#
# The result is the root's `pot` (passthru.root: the manifest; passthru.backend: the bubblewrap
# backend: bubblewrap on Linux, Seatbelt on Darwin; passthru.containerBackend: the container backend, or null).
# Kit manifests are checked when it builds (compose.py validate).
{ potSrc, containerFor }:
let potSrc0 = potSrc; in

{ pkgs
, name ? "default"
, grantsFile ? "~/.config/pot/grants.json"
, stateDir ? "~/.local/state/pot"
, contexts ? { }
, defaultContext ? "work"
, kits ? { }
, defaultInstance ? { }
, defaultAccount ? null
, agentMarkers ? [ ]
, hidden ? [ ]
, sealedProtect ? [ "**/.pi" ]
, tools ? [ ]
, bindings ? { }
, rootRepos ? [ ]
, repoKits ? { }
, inspect ? { }
, caBundle ? null
, limits ? { }
, clones ? { }
, container ? null
, defaultBackend ? null
, releaseSigners ? null
}:

let
  lib = pkgs.lib;
  # pot's programs, byte-compiled for this root's python (v2.18.0): each start (pot, its backend steps, a wrapped
  # command's) skips compiling them again; unchecked-hash, as a store path's mtime says nothing
  potSrc = pkgs.runCommand "pot-compiled" { } ''
    cp -r ${potSrc0} $out && chmod -R u+w $out
    ${pkgs.python3}/bin/python3 -m compileall -q --invalidation-mode unchecked-hash $out
  '';
  kitLib = import ../lib/kit.nix { inherit lib; };
  darwin = pkgs.stdenv.hostPlatform.isDarwin;
  python = pkgs.python3;
  # defaultAccount (v2.15.0's): deprecated in v2.16.0, gone in v2.17.0; the argument stays only to say so
  defaultInstance' = if defaultAccount == null then defaultInstance
    else throw "pot.lib.mkRoot: defaultAccount is gone (v2.17.0): defaultInstance.<kit> = \"NAME\"";
  # the floor every run gets, which a root's hidden adds to and cannot remove: agents' folders and
  # credential stores, and their configuration in a workspace
  baseHidden = [ "~/.claude" "~/.claude.json" "~/.pi/agent/auth.json" "~/.pi/agent/mcp-auth.json" "~/.codex/auth.json" "~/.local/share/opencode/auth.json" ];
  baseProtect = [ "**/.claude" "**/.pi" "**/.codex" ];
  # pot grant refuses under an agent whatever kits the root has: the floor of agent markers
  baseAgentMarkers = [ "CODEX_SANDBOX" "CLAUDECODE" ];
  # every defaultInstance.<kit> names a kit the root applies under instances, and one of them
  instancesOf = base: map (n: lib.removePrefix "${base}@" n) (lib.filter (lib.hasPrefix "${base}@") (lib.attrNames kits));
  badDefaults = lib.concatLists (lib.mapAttrsToList (base: inst:
    let have = instancesOf base; in
    lib.optional (!(lib.elem inst have))
      (if have == [ ] then "defaultInstance.${base}: the root has no kit ${base}@NAME (kits: ${lib.concatStringsSep ", " (lib.attrNames kits)})"
       else "defaultInstance.${base} = \"${inst}\": the root declares ${lib.concatMapStringsSep ", " (i: "${base}@${i}") have}"))
    defaultInstance');
  limits' = { memoryMax = "50%"; tasksMax = 2048; } // limits;
  clones' = { keepDays = 14; maxSize = "20G"; } // clones;
  badLimits = lib.attrNames (removeAttrs limits [ "memoryMax" "tasksMax" ]) ++ lib.attrNames (removeAttrs clones [ "keepDays" "maxSize" ]);
  reservedContexts = [ "explain" "why" "grant" "review" "init" "run" "serve" "runs" "doctor" ];
  badContexts = lib.filter (n: lib.elem n reservedContexts) (lib.attrNames contexts);

  # §8: the standard set, first on every run's PATH (a root adds any other command to `tools`;
  # in a repository with a pot-kits bundle, its bin/ carries the producer's own tools)
  standard = with pkgs; [ bashInteractive coreutils findutils gnugrep gnused gawk diffutils less gitMinimal which file
    tree ripgrep fd jq curl gnutar gzip unzip xz gnupatch python3 just gh ];
  inspectTools = with pkgs; lib.optionals (!darwin) [ procps iproute2 lsof systemdMinimal ];


  netwrap = pkgs.writeShellScript "pot-net" ''
    # inside a run with network: the proxy's socket as 127.0.0.1:3128, listening before the command
    # starts (0100007F:0C38 0A is 127.0.0.1:3128, LISTEN, in /proc/net/tcp)
    ${pkgs.socat}/bin/socat -lf /dev/null TCP-LISTEN:3128,bind=127.0.0.1,fork,reuseaddr UNIX-CONNECT:/run/pot/proxy.sock &
    listening() {
      local _ local state
      while read -r _ local _ state _; do [ "$local $state" != "0100007F:0C38 0A" ] || return 0; done </proc/net/tcp
      return 1
    }
    for _ in {1..300}; do
      ! listening || exec "$@"
      ${pkgs.coreutils}/bin/sleep 0.01
    done
    echo "pot: the network relay (socat) did not start" >&2
    exit 1
  '';
  bpf = pkgs.runCommand "pot-tiocsti.bpf" { nativeBuildInputs = [ (pkgs.python3.withPackages (p: [ p.seccomp ])) ]; } ''
    python3 ${potSrc}/tiocsti.py $out
  '';
  closureOf = n: roots: pkgs.runCommand n { } "cp ${pkgs.closureInfo { rootPaths = roots; }}/store-paths $out";
  # what the Linux backends share, by bwrap's config names (the container backend's config takes the same names)
  linuxConfig = {
    python = "${python}/bin/python3";
    proxy = "${potSrc}/proxy.py";
    broker = "${potSrc}/broker.py";
    shim = "${potSrc}/shim.py";
    socat = "${pkgs.socat}/bin/socat";
    sh = "${pkgs.bashInteractive}/bin/sh";
    shell = "${pkgs.bashInteractive}/bin/bash";
    env = "${pkgs.coreutils}/bin/env";
    cacert = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
    terminfo = "${pkgs.ncurses}/share/terminfo";
    tzdir = "${pkgs.tzdata}/share/zoneinfo";
    git = "${pkgs.gitMinimal}/bin/git";
    baseClosure = closureOf "pot-base-closure" [ pkgs.bashInteractive pkgs.coreutils pkgs.cacert pkgs.ncurses pkgs.tzdata ];
    netClosure = closureOf "pot-net-closure" [ netwrap ];
    # fetch-back's upload-pack, sandboxed: git's closure only
    gitClosure = closureOf "pot-git-closure" [ pkgs.gitMinimal ];
  };
  backendConfig = pkgs.writeText "pot-bwrap.json" (builtins.toJSON (linuxConfig // {
    bwrap = "${pkgs.bubblewrap}/bin/bwrap";
    inherit bpf netwrap;
    fd = "${pkgs.fd}/bin/fd";
    find = "${pkgs.findutils}/bin/find";
  }));
  backend = pkgs.writeShellScriptBin "pot-backend-bwrap" ''exec ${python}/bin/python3 -I -c 'import sys; sys.path.insert(0, "${potSrc}"); import bwrap; sys.exit(bwrap.main(sys.argv[1:]))' --config ${backendConfig} "$@"'';

  # pot phase 2: the container backend (backends/container, pot-container), when the root offers it. Its config is
  # bwrap's shared names (above) plus the engine, core's pins.py (the host's look at a write run) and the shims'
  # closure (python's and pot's, for the status.relay and git.fetch bindings); see docs/reference/container.md#inputs.
  # socat is named by its real file (bin/socat is a link to socat1, and the entry runs the file under netClosure)
  engines = [ "podman" "docker" ];
  containerKeys = [ "engine" "program" "image" ];
  badContainer = if container == null then null
    else if !(builtins.isAttrs container) then "pot.lib.mkRoot: container is { engine; program; image; }"
    else if removeAttrs container containerKeys != { } then
      "pot.lib.mkRoot: container is { engine; program; image; } (not ${lib.concatStringsSep ", " (lib.attrNames (removeAttrs container containerKeys))})"
    else if !(lib.elem (container.engine or null) engines) then
      "pot.lib.mkRoot: container.engine is \"podman\" or \"docker\" (not ${builtins.toJSON (container.engine or null)})"
    else if darwin then
      "pot.lib.mkRoot: container: the container backend binds the host's /nix/store, so it is Linux's (this root is ${pkgs.stdenv.hostPlatform.system}'s)"
    # QA B2: the root pins the engine; pot never runs one it finds on PATH (where a repository's bin, or a run, can
    # put a program of its own). podman's default is this nixpkgs' podman; docker's program is given
    else if container.engine == "docker" && (container.program or null) == null then
      "pot.lib.mkRoot: container.program: give docker's program (pkgs.docker-client, or an absolute path such as \"/run/current-system/sw/bin/docker\"): pot never runs an engine it finds on PATH"
    else if (container.program or null) != null && !(lib.isDerivation container.program)
      && !(lib.hasPrefix "/" (toString container.program)) then
      "pot.lib.mkRoot: container.program = ${builtins.toJSON (toString container.program)}: give an absolute path or a package (pot never runs an engine it finds on PATH)"
    else null;
  engineProgram = p: if lib.isDerivation p then "${p}/bin/${container.engine}" else toString p;
  enginePath = engineProgram (if (container.program or null) != null then container.program else pkgs.podman);
  containerConfig = pkgs.runCommand "pot-container.json" {
    nativeBuildInputs = [ python ];
    config = builtins.toJSON (linuxConfig // {
      inherit (container) engine;
      pins = "${potSrc}/pins.py";
      shimClosure = closureOf "pot-shim-closure" [ python potSrc ];
      ${container.engine} = enginePath;
    } // lib.optionalAttrs ((container.image or null) != null) { image = container.image; });
    passAsFile = [ "config" ];
  } ''
    python3 -I -c 'import json, os, sys
    c = json.load(open(sys.argv[1])); c["socat"] = os.path.realpath(c["socat"])
    json.dump(c, open(sys.argv[2], "w"), indent=1, sort_keys=True)' "$configPath" $out
  '';
  containerBackend = pkgs.writeShellScriptBin "pot-backend-container"
    ''exec ${containerFor pkgs}/bin/pot-container --config ${containerConfig} "$@"'';
  offered = if darwin then [ "seatbelt" ] else [ "bubblewrap" ] ++ lib.optional (container != null) "container";
  hint = lib.optionalString (container == null && !darwin) " (container = { engine = \"podman\"; } offers the container backend)";
  badBackend = lib.optional (defaultBackend != null && !(lib.elem defaultBackend offered))
      "pot.lib.mkRoot: defaultBackend = ${builtins.toJSON defaultBackend}: the root offers ${lib.concatStringsSep ", " offered}${hint}"
    ++ lib.concatLists (lib.mapAttrsToList (n: c: let b = c.backend or "auto"; in
      lib.optional (builtins.isAttrs c && b != "auto" && !(lib.elem b offered))
        "pot.lib.mkRoot: contexts.${n}: backend = ${builtins.toJSON b}: the root offers ${lib.concatStringsSep ", " offered}${hint}") contexts);

  # pot inside a run: explain and why from /run/pot/policy.json, and (v2.18.0) the wrappers' entry, which (v2.20.0)
  # asks the run's broker for a sibling; the backend stays named, as before
  inner = pkgs.writeText "pot-inner.json" (builtins.toJSON ({ inner = true; }
    // lib.optionalAttrs (!darwin) { backend = "${backend}/bin/pot-backend-bwrap"; }));
  # v2.20.0: a directory as stdin (python refuses to start with one) is /dev/null: it carries no data, and a sibling
  # never gets the caller's directory (it would reach what the sibling's own view hides)
  potInner = pkgs.writeShellScriptBin "pot" ''
    [ ! -d /dev/stdin ] || exec </dev/null
    exec ${python}/bin/python3 -I -c 'import sys; sys.path.insert(0, "${potSrc}"); import pot; pot.entry(sys.argv[1:])' ${inner} "$@"
  '';

  # macOS: the Seatbelt backend (seatbelt.py, /usr/bin/sandbox-exec). Its profile is written at start, from
  # policy.json; what a run reads of the store is the closure, as with bubblewrap. No netwrap (the run's
  # network is the proxy's port on the loopback) and no seccomp filter (the profile denies TIOCSTI).
  seatbeltConfig = pkgs.writeText "pot-seatbelt.json" (builtins.toJSON {
    python = "${python}/bin/python3";
    proxy = "${potSrc}/proxy.py";
    broker = "${potSrc}/broker.py";
    shim = "${potSrc}/shim.py";
    socat = "${pkgs.socat}/bin/socat";
    sh = "${pkgs.bashInteractive}/bin/sh";
    shell = "${pkgs.bashInteractive}/bin/bash";
    env = "${pkgs.coreutils}/bin/env";
    cacert = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
    terminfo = "${pkgs.ncurses}/share/terminfo";
    tzdir = "${pkgs.tzdata}/share/zoneinfo";
    fd = "${pkgs.fd}/bin/fd";
    find = "${pkgs.findutils}/bin/find";
    git = "${pkgs.gitMinimal}/bin/git";
    # the shims run pot's python, so the base closure holds it
    baseClosure = closureOf "pot-base-closure" [ pkgs.bashInteractive pkgs.coreutils pkgs.cacert pkgs.ncurses pkgs.tzdata python ];
    gitClosure = closureOf "pot-git-closure" [ pkgs.gitMinimal pkgs.coreutils ];
  });
  seatbelt = pkgs.writeShellScriptBin "pot-backend-seatbelt" ''exec ${python}/bin/python3 -I -c 'import sys; sys.path.insert(0, "${potSrc}"); import seatbelt; sys.exit(seatbelt.main(sys.argv[1:]))' --config ${seatbeltConfig} "$@"'';

  # kit.packages, tools' old name: gone at the source in v2.17.0
  noPackages = file: k: if builtins.isAttrs k && k ? packages
    then throw "pot: ${file}: packages is tools' old name, gone in v2.17.0: give tools" else k;
  kitManifests = lib.mapAttrs (n: k: let file = "the root (programs.pot.kits.${n})"; in
    kitLib.manifest { kit = noPackages file k; inherit file; }) kits;
  repoKitManifests = lib.mapAttrs (n: k: let file = "the root (programs.pot.repoKits.\"${n}\")"; in
    kitLib.manifest { kit = noPackages file k; inherit file; }) repoKits;
  kitRoots = lib.concatLists (lib.mapAttrsToList (_: k: kitLib.roots k) kits);
  closure = closureOf "pot-root-closure" (standard ++ tools ++ inspectTools ++ kitRoots ++ [ potInner ]
    ++ lib.optional (caBundle != null) caBundle);

  manifest = {
    version = 1;
    inherit name grantsFile stateDir defaultContext rootRepos bindings darwin agentMarkers baseHidden baseProtect baseAgentMarkers
      sealedProtect;
    defaultInstance = defaultInstance';
    limits = { memoryMax = toString limits'.memoryMax; tasksMax = toString limits'.tasksMax; };
    clones = { keepDays = clones'.keepDays; maxSize = toString clones'.maxSize; };
    contexts = contexts;
    kits = kitManifests;
    repoKits = repoKitManifests;
    hidden = hidden;
    inspect = inspect;
    standard = map toString standard;
    standardNames = lib.unique (map (p: p.meta.mainProgram or (lib.getName p)) standard);
    tools = map toString tools;
    inspectTools = map toString inspectTools;
    caBundle = if caBundle == null then null else "${caBundle}";
    releaseSigners = if releaseSigners == null then null else "${releaseSigners}";
    sshKeygen = if releaseSigners == null then null else "${pkgs.openssh}/bin/ssh-keygen";
    system = pkgs.stdenv.hostPlatform.system;
    git = "${pkgs.gitMinimal}/bin/git";
    nixpkgs = builtins.unsafeDiscardStringContext (toString pkgs.path);
    closure = "${closure}";
    potBin = "${potInner}/bin";
    shell = "bash";
    backends = if darwin then { seatbelt = { kind = "seatbelt"; program = "${seatbelt}/bin/pot-backend-seatbelt"; }; }
      else { bubblewrap = { kind = "bwrap"; program = "${backend}/bin/pot-backend-bwrap"; }; }
        // lib.optionalAttrs (container != null) { container = { kind = "container"; program = "${containerBackend}/bin/pot-backend-container";
          inherit (container) engine; inherit enginePath; }; };
    defaultBackend = if defaultBackend != null then defaultBackend else if darwin then "seatbelt" else "bubblewrap";
  } // lib.optionalAttrs darwin {
    # the startup line names what a Seatbelt run lacks that a Linux one has (design §12.2); explain lists every lack
    usualGuarantees = [ "own-netns" "own-pidns" ];
  };
  rootJson = pkgs.runCommand "pot-root-${name}.json" {
    nativeBuildInputs = [ python ];
    manifest = builtins.toJSON manifest;
    passAsFile = [ "manifest" ];
  } ''
    python3 -I ${potSrc}/compose.py validate-root "$manifestPath"
    cp "$manifestPath" $out
  '';
in
if badContexts != [ ] then throw "pot: mkRoot: a context may not be named like a pot verb: ${lib.concatStringsSep ", " badContexts}"
else if badLimits != [ ] then throw "pot.lib.mkRoot: limits is { memoryMax; tasksMax; } and clones { keepDays; maxSize; } (not ${lib.concatStringsSep ", " badLimits})"
else if badDefaults != [ ] then throw "pot.lib.mkRoot: ${lib.concatStringsSep "; " badDefaults}"
else if badContainer != null then throw badContainer
else if badBackend != [ ] then throw (lib.concatStringsSep "; " badBackend)
# share/pot/run-1 (v2.18.0): this pot takes `pot run`, the wrappers' entry; a wrapper looks for it beside the
# launcher's bin/pot, and runs its command unwrapped (with one line) under an older root's pot
else pkgs.runCommand "pot" {
  passthru = { root = manifest; inherit rootJson; backend = if darwin then seatbelt else backend;
    containerBackend = if container != null then containerBackend else null; };
  meta = { description = "pot: run a command under the root's policy (${name})"; mainProgram = "pot"; };
} ''
  mkdir -p $out/bin $out/share/pot
  cat > $out/bin/pot <<'EOF'
  #!${pkgs.runtimeShell}
  exec ${python}/bin/python3 -I -c 'import sys; sys.path.insert(0, "${potSrc}"); import pot; pot.entry(sys.argv[1:])' ${rootJson} "$@"
  EOF
  chmod +x $out/bin/pot
  echo "pot run --command NAME --manifest PATH -- ARGS" > $out/share/pot/run-1

''
