{
  description = "execfilter — LD_PRELOAD shim that strips nixGL/glibc-only env vars from non-nix children";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        rec {
          execfilter = pkgs.rustPlatform.buildRustPackage {
            pname = "execfilter";
            version = "0.1.0";
            src = self;
            cargoLock.lockFile = ./Cargo.lock;
            meta = with pkgs.lib; {
              description = "LD_PRELOAD shim filtering nixGL env vars for non-nix children";
              license = licenses.isc;
              platforms = [
                "x86_64-linux"
                "aarch64-linux"
              ];
            };
          };
          default = execfilter;
        }
      );
    };
}
