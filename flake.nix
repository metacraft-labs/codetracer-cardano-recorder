{
  description = "CodeTracer Aiken/Cardano Recorder";

  nixConfig = {
    extra-substituters = [
      "https://cache.metacraft-labs.com/metacraft-public"
    ];
    extra-trusted-public-keys = [
      "metacraft-public:UtS6PK+p0uZaJK3i/jD2DQOjTpddhQUQmNQDQih5N4Q="
    ];
  };

  inputs = {
    mcl-blockchain.url = "github:metacraft-labs/nix-blockchain-development";
    nixpkgs.follows = "mcl-blockchain/nixpkgs";
    flake-utils.follows = "mcl-blockchain/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      mcl-blockchain,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        devShells.default = pkgs.mkShell {
          inputsFrom = [ mcl-blockchain.devShells.${system}.aiken ];
          packages = [
            pkgs.zstd # required by libcodetracer_trace_writer (Nim FFI)
            # Declare nim + nimble + just + capnproto explicitly
            # so the dev shell is self-contained.  Previously these
            # came via inputsFrom = [aiken], but the cached aiken
            # devShell on the mcl-blockchain cachix substituter
            # sometimes resolves to a build whose PATH is missing
            # nimble (the binary IS in the store, but the cached
            # devShell's drvAttrs.nativeBuildInputs were partial).
            # Declaring them directly here keeps the contract
            # explicit and removes the substituter-keyed surprise.
            pkgs.nim
            pkgs.nimble
            pkgs.just
            pkgs.capnproto
            pkgs.rustc
            pkgs.cargo
            pkgs.rustfmt
            pkgs.clippy
            pkgs.pkg-config
          ];
        };
      }
    );
}
