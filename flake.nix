{
  description = "xmonad-style tiling window manager for the river Wayland compositor";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ rust-overlay.overlays.default ];
            }
          )
        );

      oxmonadFor =
        pkgs:
        let
          toolchain = pkgs.rust-bin.stable.latest.minimal;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          };
        in
        pkgs.callPackage (
          {
            lib,
            river,
            swaylock,
          }:
          rustPlatform.buildRustPackage {
            pname = "oxmonad";
            version = (lib.importTOML ./Cargo.toml).workspace.package.version;

            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./crates
                ./assets
                ./contrib
              ];
            };

            cargoLock.lockFile = ./Cargo.lock;
            buildType = "deploy";
            cargoBuildFlags = [
              "-p"
              "oxmonad"
            ];

            # The test suite needs system fonts and a dbus-daemon, which the
            # build sandbox does not provide; run `cargo test` in the dev shell.
            doCheck = false;

            postInstall = ''
              install -Dm755 contrib/oxmonad-lock $out/bin/oxmonad-lock
              wrapProgram $out/bin/oxmonad-lock \
                --suffix PATH : ${lib.makeBinPath [ swaylock ]}

              substitute contrib/lightdm/oxmonad-session $out/bin/oxmonad-session \
                --replace-fail "exec river -c /usr/local/bin/oxmonad" \
                  "exec ${lib.getExe river} -c $out/bin/oxmonad"
              chmod 755 $out/bin/oxmonad-session

              substitute contrib/lightdm/oxmonad.desktop \
                $out/share/wayland-sessions/oxmonad.desktop \
                --replace-fail /usr/local/bin/oxmonad-session $out/bin/oxmonad-session
            '';

            nativeBuildInputs = [ pkgs.makeWrapper ];

            preInstall = ''
              mkdir -p $out/share/wayland-sessions
            '';

            passthru.providedSessions = [ "oxmonad" ];

            meta = {
              description = "xmonad-style tiling window manager for the river Wayland compositor";
              license = lib.licenses.mit;
              mainProgram = "oxmonad";
              platforms = lib.platforms.linux;
            };
          }
        ) { };
    in
    {
      packages = forAllSystems (pkgs: rec {
        oxmonad = oxmonadFor pkgs;
        default = oxmonad;
      });

      overlays.default = final: _prev: {
        oxmonad = self.packages.${final.stdenv.hostPlatform.system}.oxmonad;
      };

      nixosModules.default =
        {
          config,
          lib,
          pkgs,
          ...
        }:
        let
          cfg = config.programs.oxmonad;
        in
        {
          options.programs.oxmonad = {
            enable = lib.mkEnableOption "oxmonad, an xmonad-style window manager for river";
            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.oxmonad;
              defaultText = lib.literalExpression "oxmonad.packages.\${system}.oxmonad";
              description = "The oxmonad package to use.";
            };
          };

          config = lib.mkIf cfg.enable {
            environment.systemPackages = [
              cfg.package
              pkgs.river
            ];
            services.displayManager.sessionPackages = [ cfg.package ];
            # river needs a seat and a GPU; swaylock (used by oxmonad-lock)
            # needs a PAM service.
            hardware.graphics.enable = lib.mkDefault true;
            security.polkit.enable = lib.mkDefault true;
            security.pam.services.swaylock = { };
          };
        };

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = [
            (pkgs.rust-bin.stable.latest.default.override {
              extensions = [
                "rust-src"
                "rust-analyzer"
              ];
            })
            pkgs.river
            pkgs.dbus
          ];
        };
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt);
    };
}
