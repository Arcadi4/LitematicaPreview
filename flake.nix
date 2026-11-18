{
  description = "Litematica Preview";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/f4a03c008967841f2a0b3cb7c436fc6fc31671f6";

  outputs = {
    self,
    nixpkgs,
  }: let
    lib = nixpkgs.lib;
    systems = ["x86_64-linux"];
    cargoManifest = builtins.fromTOML (builtins.readFile ./src-tauri/Cargo.toml);
    version = cargoManifest.package.version;
    pnpmHash = "sha256-86yhmjdMEb05DB8AvhPZNzFLVLTvPd7PEh9XrthVhqQ=";

    cleanSource = src:
      lib.cleanSourceWith {
        inherit src;
        filter = path: type: let
          excluded = [
            ".codegraph"
            ".git"
            ".vite-hooks"
            "artifacts"
            "dist"
            "node_modules"
            "result"
            "target"
          ];
        in
          !(builtins.elem (builtins.baseNameOf path) excluded);
      };

    mkFrontend = {
      pkgs,
      src,
    }: let
      pnpm = pkgs.pnpm_12;
      pnpmDeps = pkgs.fetchPnpmDeps {
        pname = "litematica-preview-frontend";
        inherit version src pnpm;
        fetcherVersion = 5;
        hash = pnpmHash;
      };
    in
      pkgs.stdenv.mkDerivation {
        pname = "litematica-preview-frontend";
        inherit version src pnpmDeps;

        nativeBuildInputs = [
          pkgs.nodejs_24
          pnpm
          pkgs.pnpmConfigHook
        ];

        pnpmRoot = ".";

        buildPhase = ''
          runHook preBuild
          pnpm run build
          runHook postBuild
        '';

        installPhase = ''
          runHook preInstall
          mkdir -p "$out"
          cp -R dist/. "$out/"
          runHook postInstall
        '';

        meta = {
          description = "Litematica Preview frontend bundle";
          platforms = systems;
        };
      };

    mkApp = {
      pkgs,
      src,
      frontend,
    }:
      pkgs.rustPlatform.buildRustPackage {
        pname = "litematica-preview";
        inherit version src;
        cargoLock = {lockFile = ./Cargo.lock;};

        cargoRoot = ".";
        cargoBuildFlags = [
          "-p"
          "litematica-preview"
          "--bin"
          "LitematicaPreview"
          "--locked"
        ];
        doCheck = false;

        nativeBuildInputs = [
          pkgs.desktop-file-utils
          pkgs.pkg-config
          pkgs.shared-mime-info
          pkgs.wrapGAppsHook3
        ];

        buildInputs = [
          pkgs.glib
          pkgs.glib-networking
          pkgs.gtk3
          pkgs.libayatana-appindicator
          pkgs.librsvg
          pkgs.mesa
          pkgs.openssl
          pkgs.vulkan-loader
          pkgs.webkitgtk_4_1
          pkgs.xdo
        ];

        preBuild = ''
          mkdir -p dist
          cp -R ${frontend}/. dist/
        '';

        installPhase = ''
          runHook preInstall

          mkdir -p "$out/bin" "$out/lib/Litematica Preview"
          install -m755 \
            "target/${pkgs.stdenv.hostPlatform.rust.rustcTargetSpec}/release/LitematicaPreview" \
            "$out/bin/LitematicaPreview"

          cp -R Assets "$out/lib/Litematica Preview/Assets"
          cp -R Fixtures/Demos "$out/lib/Litematica Preview/Demos"

          mkdir -p "$out/lib/Litematica Preview/Licenses"
          cp LICENSE NOTICE "$out/lib/Litematica Preview/Licenses/"
          cp -R ThirdParty "$out/lib/Litematica Preview/Licenses/ThirdParty"

          mkdir -p "$out/share/applications"
          printf '%s\n' \
            '[Desktop Entry]' \
            'Type=Application' \
            'Name=Litematica Preview' \
            'Comment=Offline Minecraft schematic viewer' \
            'Exec=LitematicaPreview %f' \
            'Icon=dev.litematica.preview' \
            'Terminal=false' \
            'Categories=Utility;' \
            'MimeType=application/x-litematic;application/x-schematic;application/x-nbt;application/x-snbt;application/x-mcstructure;application/x-mca;application/x-nusn;' \
            'StartupWMClass=LitematicaPreview' \
            > "$out/share/applications/dev.litematica.preview.desktop"

          mkdir -p "$out/share/icons/hicolor/1024x1024/apps"
          cp Assets/icon.png \
            "$out/share/icons/hicolor/1024x1024/apps/dev.litematica.preview.png"

          mkdir -p "$out/share/mime/packages"
          cp src-tauri/linux/dev.litematica.preview.xml \
            "$out/share/mime/packages/dev.litematica.preview.xml"

          runHook postInstall
        '';

        meta = {
          description = "Offline Minecraft schematic and structure viewer";
          homepage = "https://github.com/Arcadi4/LitematicaPreview";
          license = lib.licenses.agpl3Only;
          mainProgram = "LitematicaPreview";
          platforms = systems;
        };
      };

    mkDevShell = {pkgs}:
      pkgs.mkShell {
        packages = [
          pkgs.cargo
          pkgs.clippy
          pkgs.nodejs_24
          pkgs.patchelf
          pkgs.pkg-config
          pkgs.pnpm_12
          pkgs.rustc
          pkgs.rustfmt
        ];

        buildInputs = [
          pkgs.glib
          pkgs.glib-networking
          pkgs.gtk3
          pkgs.libayatana-appindicator
          pkgs.librsvg
          pkgs.mesa
          pkgs.openssl
          pkgs.vulkan-loader
          pkgs.webkitgtk_4_1
          pkgs.xdo
        ];
      };

    perSystem = lib.genAttrs systems (system: let
      pkgs = import nixpkgs {inherit system;};
      src = cleanSource self;
      frontend = mkFrontend {inherit pkgs src;};
      app = mkApp {inherit pkgs src frontend;};
    in {
      packages = {
        inherit frontend;
        default = app;
      };

      apps.default = {
        type = "app";
        program = "${app}/bin/LitematicaPreview";
      };

      devShells.default = mkDevShell {inherit pkgs;};
      formatter = pkgs.alejandra;
    });
  in {
    packages = lib.mapAttrs (_: value: value.packages) perSystem;
    apps = lib.mapAttrs (_: value: value.apps) perSystem;
    devShells = lib.mapAttrs (_: value: value.devShells) perSystem;
    formatter = lib.mapAttrs (_: value: value.formatter) perSystem;
  };
}
