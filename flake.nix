{
  description = "CodeTracer Aiken/Cardano Recorder";

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
            # Portable mcl-standard-hooks rules are committed separately.
            # This owning locked shell supplies their native executables.
            pkgs.prek
            pkgs.uv
            pkgs.editorconfig-checker
            pkgs.nixfmt-rfc-style
            pkgs.opentofu
            pkgs.nodePackages.prettier
            pkgs.python3
            pkgs.zstd # required by libcodetracer_trace_writer (Nim FFI)
            # Declare nim + nimble + just + capnproto explicitly
            # so the dev shell is self-contained.  Previously these
            # came via inputsFrom = [aiken], but the cached aiken
            # devShell on the mcl-blockchain Attic substituter
            # sometimes resolves to a build whose PATH is missing
            # nimble (the binary IS in the store, but the cached
            # devShell's drvAttrs.nativeBuildInputs were partial).
            # Declaring them directly here keeps the contract
            # explicit and removes the substituter-keyed surprise.
            pkgs.nim
            pkgs.nimble
            # `git` from nixpkgs, ahead of the host's. On macOS the host's
            # `/usr/bin/git` is an xcode-select trampoline that runs
            # `$DEVELOPER_DIR/usr/bin/xcrun`; in this shell DEVELOPER_DIR is the
            # nixpkgs apple-sdk, whose xcrun (xcbuild) prints "warning: unhandled
            # Platform key FamilyDisplayName" on every call. nimble reads git's
            # stderr together with its stdout, so with that git `nimble install` would
            # reject `git rev-parse HEAD` as "not a valid sha1 hash value".
            pkgs.git
            pkgs.just
            pkgs.capnproto
            pkgs.rustc
            pkgs.cargo
            pkgs.rustfmt
            pkgs.clippy
            pkgs.pkg-config
          ];

          # `cargo <subcommand>` looks for `cargo-<subcommand>` in
          # `$CARGO_HOME/bin` BEFORE it searches PATH. On any machine with
          # rustup — including the self-hosted macOS runner — that directory
          # holds rustup's proxies, so `cargo fmt` and `cargo clippy` run
          # rustup's `cargo-fmt` / `cargo-clippy` instead of the rustfmt and
          # clippy above, and fail with "'cargo-fmt' is not installed for the
          # toolchain".
          #
          # The shell therefore gets its own CARGO_HOME with an empty `bin/`,
          # so subcommand lookup falls through to PATH. `registry/` and `git/`
          # are symlinks to the real CARGO_HOME, and so are its config and
          # credentials when present: the download cache is shared, and only
          # the proxy directory is left behind.
          shellHook = ''
            # Execute the configured upstream hooks faithfully; Prek0.2.17
            # native size rounding differs at the exact 1 MiB boundary.
            export PREK_NO_FAST_PATH=1
            _ct_real_cargo_home="''${CARGO_HOME:-$HOME/.cargo}"
            _ct_cargo_home="''${XDG_CACHE_HOME:-$HOME/.cache}/codetracer-cardano-recorder/cargo-home"
            if [ "$_ct_real_cargo_home" != "$_ct_cargo_home" ]; then
              mkdir -p "$_ct_cargo_home" \
                "$_ct_real_cargo_home/registry" "$_ct_real_cargo_home/git"
              # Re-pointed on every entry, so a changed CARGO_HOME is followed
              # rather than left sharing the previous one's cache. Only a link
              # is ever replaced; a real file placed here is left alone.
              for _ct_entry in registry git config.toml credentials.toml; do
                if [ -e "$_ct_real_cargo_home/$_ct_entry" ] &&
                  { [ -L "$_ct_cargo_home/$_ct_entry" ] ||
                    [ ! -e "$_ct_cargo_home/$_ct_entry" ]; }; then
                  ln -sfn "$_ct_real_cargo_home/$_ct_entry" "$_ct_cargo_home/$_ct_entry"
                fi
              done
              export CARGO_HOME="$_ct_cargo_home"
            fi
            unset _ct_real_cargo_home _ct_cargo_home _ct_entry
          '';
        };
      }
    );
}
