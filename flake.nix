{
  description = "skald — a CLI for ticket stores";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "skald";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          # `docket` was the runner-up name and is fully self-explanatory — "a register of matters
          # awaiting action" — so it ships as a supported second name. A symlink rather than a second
          # `[[bin]]`, because two bin targets would compile the same crate twice for no gain.
          # `skald` stays canonical in the docs so there is one name in the corpus.
          postInstall = ''
            ln -s skald $out/bin/docket
          '';

          meta = with pkgs.lib; {
            description = "The only read/write path to a ticket store";
            mainProgram = "skald";
            license = licenses.mit;
            platforms = platforms.unix ++ platforms.windows;
          };
        };

        apps.default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/skald";
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            clippy
            rustc
            rustfmt
          ];
        };
      }
    );
}
