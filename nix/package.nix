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
  doCheck = false;

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
