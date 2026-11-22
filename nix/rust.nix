# pot's Rust code, built with crane as grove's dev kit does: the package, and checks for the build, the tests
# (unit tests and the trycmd transcripts), clippy (pedantic, warnings denied), rustfmt and cargo-deny.
{ crane }:
pkgs:
let
  inherit (pkgs) lib;
  craneLib = crane.mkLib pkgs;
  root = ./..;
  # The Cargo workspace, and what its tests read: the transcripts and their run folders, policy-1's fixtures and schema
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (craneLib.fileset.commonCargoSources root)
      ../deny.toml
      ../docs/reference/container
      ../contracts/fixtures/policy-1
      ../contracts/policy-1.schema.json
    ];
  };
  # pname and version from the member's Cargo.toml (the workspace's root manifest has neither)
  crate = craneLib.crateNameFromCargoToml { cargoToml = ../backends/container/Cargo.toml; };
  commonArgs = {
    inherit src;
    inherit (crate) pname version;
    strictDeps = true;
  };
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
  package = craneLib.buildPackage (commonArgs // {
    inherit cargoArtifacts;
    doCheck = false; # the tests run once, in checks.pot-container-test
    meta.mainProgram = "pot-container";
  });
in
{
  inherit package;
  checks = {
    pot-container = package;
    pot-container-test = craneLib.cargoTest (commonArgs // { inherit cargoArtifacts; });
    pot-container-clippy = craneLib.cargoClippy (commonArgs // {
      inherit cargoArtifacts;
      cargoClippyExtraArgs = "--all-targets -- --deny warnings";
    });
    pot-container-fmt = craneLib.cargoFmt { inherit src; inherit (crate) pname version; };
    # Advisories need the network, which the build sandbox does not have: run `cargo deny check advisories`
    # in `nix develop`.
    pot-container-deny = craneLib.cargoDeny {
      inherit src;
      inherit (crate) pname version;
      cargoDenyChecks = "bans licenses sources";
    };
  };
}
