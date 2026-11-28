{
  description = "pot's development checks: inputs that pot's own flake keeps out of every root's and producer's lock";

  # The docs check reads the whole repository (../), so run it from pot's checkout as a git flake, never path:
  # (that would copy dev/ alone):
  #   nix flake check ./dev
  # knowledge-base-kit is read from this input alone (pot grafts nothing). To check against an unreleased edit:
  #   nix flake check ./dev --override-input knowledge-base-kit path:PATH/TO/knowledge-base-kit
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/5b31ff946309736077f0c4ee23bb85d0a0a1c181";   # pot's nixos-26.05
  inputs.knowledge-base-kit = { url = "github:danielnaab/knowledge-base-kit/v2.1.0"; flake = false; };

  outputs = { nixpkgs, knowledge-base-kit, ... }:
    let
      lib = nixpkgs.lib;
      forAll = f: lib.genAttrs [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ]
        (system: f nixpkgs.legacyPackages.${system});
      repo = builtins.path { path = ./..; name = "pot-source"; };
    in
    {
      checks = forAll (pkgs: {
        # pot's docs against knowledge-base-kit's conventions: links and anchors, lifecycle status,
        # provenance (scopes in knowledge-base.yaml). The checker is knowledge-base-kit's src/.
        docs = pkgs.runCommand "pot-docs" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          cd ${repo}
          export PYTHONPATH=${knowledge-base-kit}/src
          # one command per line: under set -e, a failure left of `&&` would not stop the build
          python3 -m link_validator .
          python3 -m kb_linter .
          touch $out
        '';
      });
    };
}
