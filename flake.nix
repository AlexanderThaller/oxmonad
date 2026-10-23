{
  description = "xmonad-style tiling window manager for the river Wayland compositor";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      crane,
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
          craneLib = (crane.mkLib pkgs).overrideToolchain (p: p.rust-bin.stable.latest.minimal);
        in
        pkgs.callPackage (
          {
            lib,
            dbus,
            river,
            swaylock,
            systemd,
          }:
          let
            commonArgs = {
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

              strictDeps = true;
              CARGO_PROFILE = "deploy";
              cargoExtraArgs = "--locked -p oxmonad";

              # The test suite needs system fonts and a dbus-daemon, which the
              # build sandbox does not provide; run `cargo test` in the dev shell.
              doCheck = false;
            };

            # The dependencies on their own, so a change to oxmonad's sources
            # only rebuilds the workspace crates. Rebuilt when Cargo.lock or
            # the toolchain changes.
            cargoArtifacts = craneLib.buildDepsOnly (
              commonArgs
              // {
                # Only `cargo build`: the default extra `cargo check` adds
                # nothing to the release build.
                buildPhaseCargoCommand = "cargoWithProfile build ${commonArgs.cargoExtraArgs}";
              }
            );
          in
          craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;

              postInstall = ''
                install -Dm755 contrib/oxmonad-lock $out/bin/oxmonad-lock
                wrapProgram $out/bin/oxmonad-lock \
                  --suffix PATH : ${lib.makeBinPath [ swaylock ]}

                substitute contrib/lightdm/oxmonad-session $out/bin/oxmonad-session \
                  --replace-fail "oxmonad=/usr/local/bin/oxmonad" "oxmonad=$out/bin/oxmonad"
                chmod 755 $out/bin/oxmonad-session
                wrapProgram $out/bin/oxmonad-session \
                  --prefix PATH : ${
                    lib.makeBinPath [
                      river
                      dbus
                      systemd
                    ]
                  }

                install -Dm644 contrib/systemd/oxmonad-session.target \
                  $out/lib/systemd/user/oxmonad-session.target

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
          )
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
            # oxmonad-session.target, started by the session wrapper.
            systemd.packages = [ cfg.package ];
            # river needs a seat and a GPU; swaylock (used by oxmonad-lock)
            # needs a PAM service.
            hardware.graphics.enable = lib.mkDefault true;
            security.polkit.enable = lib.mkDefault true;
            security.pam.services.swaylock = { };
          };
        };

      # Manages ~/.config/oxmonad/config.toml. Use it next to the NixOS module,
      # which provides the river session.
      homeManagerModules.default =
        {
          config,
          lib,
          pkgs,
          ...
        }:
        let
          cfg = config.programs.oxmonad;
          toml = pkgs.formats.toml { };
          configFile = toml.generate "oxmonad-config.toml" cfg.settings;
          # Fails the build when oxmonad rejects the file, instead of the
          # running oxmonad keeping the old config after a reload.
          checkedConfigFile = pkgs.runCommand "oxmonad-config.toml" { } ''
            ${lib.getExe cfg.package} --config ${configFile} --check-config
            cp ${configFile} $out
          '';
        in
        {
          options.programs.oxmonad = {
            enable = lib.mkEnableOption "oxmonad, an xmonad-style window manager for river";
            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.oxmonad;
              defaultText = lib.literalExpression "oxmonad.packages.\${system}.oxmonad";
              description = "The oxmonad package to install and to check the configuration with.";
            };
            settings = lib.mkOption {
              inherit (toml) type;
              default = { };
              example = lib.literalExpression ''
                {
                  modifier = "Super";
                  border.focused = "#FF8C00";
                  keybindings."Mod+p" = "spawn fuzzel";
                }
              '';
              description = ''
                Written to {file}`$XDG_CONFIG_HOME/oxmonad/config.toml` and merged
                over the built-in defaults by oxmonad, so only set what you change.
                See docs/config.adoc for every setting. No file is written when empty.
              '';
            };
            checkConfig = lib.mkOption {
              type = lib.types.bool;
              default = true;
              description = "Validate the configuration with `oxmonad --check-config` at build time.";
            };
            reloadOnChange = lib.mkOption {
              type = lib.types.bool;
              default = true;
              description = "Send SIGUSR1 to a running oxmonad when the configuration changes, so it reloads the file.";
            };
          };

          config = lib.mkIf cfg.enable {
            home.packages = [ cfg.package ];
            xdg.configFile."oxmonad/config.toml" = lib.mkIf (cfg.settings != { }) {
              source = if cfg.checkConfig then checkedConfigFile else configFile;
              onChange = lib.mkIf cfg.reloadOnChange ''
                ${pkgs.procps}/bin/pkill -USR1 -x oxmonad || true
              '';
            };
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
