{
  description = "Barestash native Rust CLI development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { nixpkgs, ... }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-darwin"
        "x86_64-linux"
      ];

      forAllSystems =
        callback:
        builtins.listToAttrs (
          map (
            system:
            let
              pkgs = import nixpkgs { inherit system; };
            in
            {
              name = system;
              value = callback pkgs;
            }
          ) systems
        );
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages =
            with pkgs;
            [
              cargo
              clippy
              deadnix
              git
              gh
              just
              nixfmt
              ripgrep
              rustc
              rustfmt
              statix
            ]
            ++ lib.optionals stdenv.isLinux [ dbus ];
        };
      });

      formatter = forAllSystems (
        pkgs:
        pkgs.writeShellApplication {
          name = "nixfmt";
          runtimeInputs = [ pkgs.nixfmt ];
          text = ''
            if [ "$#" -eq 0 ]; then
              exec nixfmt flake.nix
            fi

            exec nixfmt "$@"
          '';
        }
      );

      checks = forAllSystems (pkgs: {
        nix-quality =
          pkgs.runCommandLocal "barestash-cli-nix-quality"
            {
              nativeBuildInputs = with pkgs; [
                deadnix
                nixfmt
                statix
              ];
              src = ./.;
            }
            ''
              cp -R "$src" source
              chmod -R +w source
              cd source

              nixfmt --check flake.nix
              statix check flake.nix
              deadnix --fail flake.nix

              touch "$out"
            '';
      });
    };
}
