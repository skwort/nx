{
  coreutils,
  lib,
  makeWrapper,
  nix,
  rustPlatform,
}:

rustPlatform.buildRustPackage {
  pname = "nx";
  version = "0.1.0";

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.lock
      ../Cargo.toml
      ../src
      ../tests
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  nativeBuildInputs = [ makeWrapper ];

  preCheck = ''
    export XDG_CACHE_HOME="$TMPDIR/cache"
    export XDG_CONFIG_HOME="$TMPDIR/config"
    export XDG_RUNTIME_DIR="$TMPDIR/runtime"
    mkdir -p "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME" "$XDG_RUNTIME_DIR"
  '';

  postInstall = ''
    for program in nx nxd; do
      wrapProgram "$out/bin/$program" \
        --prefix PATH : ${
          lib.makeBinPath [
            coreutils
            nix
          ]
        }
    done
  '';

  meta = {
    description = "Helper application for NixOS";
    homepage = "https://github.com/skwort/nx";
    mainProgram = "nx";
    platforms = lib.platforms.linux;
  };
}
