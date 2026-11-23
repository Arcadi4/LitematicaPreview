{
  description = "Litematica Preview";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/f4a03c008967841f2a0b3cb7c436fc6fc31671f6";

  outputs = {self, nixpkgs}: let
    system = "x86_64-linux";
    pkgs = import nixpkgs {inherit system;};
    inherit (pkgs) lib;
    version = (builtins.fromTOML (builtins.readFile ./app/Cargo.toml)).package.version;
    windowLibraries = with pkgs; [
      fontconfig
      libxkbcommon
      vulkan-loader
      wayland
      xorg.libX11
      xorg.libXcursor
      xorg.libXi
      xorg.libXrandr
      xorg.libxcb
    ];
    app = pkgs.rustPlatform.buildRustPackage {
      pname = "litematica-preview";
      inherit version;
      src = self;
      cargoLock = {
        lockFile = ./Cargo.lock;
        allowBuiltinFetchGit = true;
      };
      cargoBuildFlags = ["-p" "litematica-preview" "--bin" "LitematicaPreview" "--features" "managed-install"];
      cargoTestFlags = ["-p" "litematica-preview" "--features" "managed-install"];
      nativeBuildInputs = with pkgs; [makeWrapper pkg-config];
      buildInputs = windowLibraries;
      postInstall = ''
        install -Dm644 app/linux/litematica-preview.desktop "$out/share/applications/dev.litematica.preview.desktop"
        install -Dm644 app/linux/dev.litematica.preview.xml "$out/share/mime/packages/dev.litematica.preview.xml"
        install -Dm644 Assets/icon.png "$out/share/icons/hicolor/1024x1024/apps/dev.litematica.preview.png"
        install -Dm644 LICENSE "$out/share/licenses/litematica-preview/LICENSE"
        install -Dm644 app/icons/LICENSE "$out/share/licenses/litematica-preview/Fluent-UI-System-Icons-LICENSE"
        wrapProgram "$out/bin/LitematicaPreview" \
          --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath windowLibraries}
      '';
      meta = {
        description = "Offline Minecraft schematic and structure viewer";
        homepage = "https://github.com/Arcadi4/LitematicaPreview";
        license = lib.licenses.agpl3Only;
        mainProgram = "LitematicaPreview";
        platforms = [system];
      };
    };
  in {
    packages.${system}.default = app;
    apps.${system}.default = {
      type = "app";
      program = "${app}/bin/LitematicaPreview";
    };
    devShells.${system}.default = pkgs.mkShell {
      packages = with pkgs; [cargo clippy just pkg-config rustc rustfmt];
      buildInputs = windowLibraries;
    };
    formatter.${system} = pkgs.alejandra;
  };
}
