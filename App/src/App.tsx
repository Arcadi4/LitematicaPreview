import { useCallback, useEffect, useRef, useState } from "react";
import {
  Badge,
  Button,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  FluentProvider,
  Menu,
  MenuDivider,
  MenuGroup,
  MenuGroupHeader,
  MenuItem,
  MenuItemRadio,
  MenuList,
  MenuPopover,
  MenuTrigger,
  MessageBar,
  MessageBarActions,
  MessageBarBody,
  MessageBarTitle,
  ToggleButton,
  Tooltip,
  webDarkTheme,
  webLightTheme,
} from "@fluentui/react-components";
import {
  Add20Regular,
  ArrowExpand20Regular,
  ArrowRight20Regular,
  Cube24Regular,
  Dismiss20Regular,
  Document20Regular,
  FolderOpen20Regular,
  Grid20Regular,
  Home20Regular,
  Info20Regular,
  Keyboard20Regular,
  MoreHorizontal20Regular,
  ShieldCheckmark20Regular,
  Subtract20Regular,
} from "@fluentui/react-icons";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import icon from "../../Assets/app.ico?url";
import { SchematicRenderer, type PreviewMetadata } from "./renderer";

type Bootstrap = {
  extensions: string[];
  demos: { name: string; path: string; extension: string }[];
  initialPath: string | null;
  version: string;
  requestId: number;
};
type ThemePreference = "system" | "light" | "dark";
type Loading = { path: string; phase: "decode" | "upload" };
type Loaded = { path: string; metadata: PreviewMetadata; seconds: number };
type Notice = { intent: "error" | "success" | "info"; message: string };

const appName = "Litematica Preview";
const numbers = new Intl.NumberFormat();
const dimensions = new Intl.NumberFormat(undefined, {
  maximumFractionDigits: 2,
});
const fileName = (path: string) => path.split(/[\\/]/).pop() || path;
const errorMessage = (error: unknown) =>
  error instanceof Error ? error.message : String(error);
const isTheme = (value: unknown): value is ThemePreference =>
  value === "system" || value === "light" || value === "dark";

function savedTheme(): ThemePreference {
  try {
    const value = localStorage.getItem("litematica-preview-theme");
    return isTheme(value) ? value : "system";
  } catch {
    return "system";
  }
}

export default function App() {
  const [bootstrap, setBootstrap] = useState<Bootstrap | null>(null);
  const [loading, setLoading] = useState<Loading | null>(null);
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [dragging, setDragging] = useState(false);
  const [grid, setGrid] = useState(true);
  const [theme, setTheme] = useState<ThemePreference>(savedTheme);
  const [systemDark, setSystemDark] = useState(
    () => matchMedia("(prefers-color-scheme: dark)").matches,
  );
  const [dialog, setDialog] = useState<"controls" | "about" | null>(null);
  const [actionBusy, setActionBusy] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rendererRef = useRef<SchematicRenderer | null>(null);
  const bootstrapRef = useRef<Bootstrap | null>(null);
  const gridRef = useRef(true);
  const mounted = useRef(false);
  const generation = useRef(0);
  const initialPathConsumed = useRef(false);
  const choosingRef = useRef(false);
  const actionBusyRef = useRef(false);
  const titleQueue = useRef<Promise<void>>(Promise.resolve());

  const isCurrent = useCallback(
    (id: number) => mounted.current && generation.current === id,
    [],
  );

  // Serialize title changes so a delayed native call cannot restore an old filename.
  const updateTitle = useCallback(
    (path: string | null, id: number) => {
      titleQueue.current = titleQueue.current.then(async () => {
        if (!isCurrent(id)) return;
        try {
          await getCurrentWindow().setTitle(
            path ? `${fileName(path)} — ${appName}` : appName,
          );
        } catch (error) {
          if (isCurrent(id))
            setNotice({
              intent: "error",
              message: `Could not update the window title: ${errorMessage(error)}`,
            });
        }
      });
    },
    [isCurrent],
  );

  const cancelNative = useCallback(
    (id: number) => {
      void invoke("cancel_load", { requestId: id }).catch((error: unknown) => {
        if (isCurrent(id))
          setNotice({
            intent: "error",
            message: `Could not cancel the load: ${errorMessage(error)}`,
          });
      });
    },
    [isCurrent],
  );

  const clearView = useCallback(() => {
    rendererRef.current?.clear();
    setLoaded(null);
    setLoading(null);
    setDragging(false);
  }, []);

  const home = useCallback(() => {
    const id = ++generation.current;
    cancelNative(id);
    clearView();
    setNotice(null);
    updateTitle(null, id);
  }, [cancelNative, clearView, updateTitle]);

  const graphicsFailed = useCallback(
    (message: string) => {
      if (!mounted.current) return;
      const id = ++generation.current;
      cancelNative(id);
      const renderer = rendererRef.current;
      rendererRef.current = null;
      renderer?.dispose();
      setLoaded(null);
      setLoading(null);
      setNotice({
        intent: "error",
        message: `The preview could not be rendered. ${message}`,
      });
      updateTitle(null, id);
    },
    [cancelNative, updateTitle],
  );

  const loadPath = useCallback(
    async (path: string) => {
      if (!mounted.current) return;
      const id = ++generation.current;
      clearView();
      setNotice(null);
      updateTitle(null, id);
      const supported = bootstrapRef.current?.extensions.some((extension) =>
        path.toLowerCase().endsWith(extension.toLowerCase()),
      );
      if (!supported) {
        cancelNative(id);
        setNotice({
          intent: "error",
          message: `“${fileName(path)}” is not a supported schematic. Choose one of the formats listed below.`,
        });
        return;
      }
      setLoading({ path, phase: "decode" });
      const started = performance.now();
      try {
        // load_preview advances the native generation itself; an older IPC call can never supersede it.
        const buffer = await invoke<ArrayBuffer>("load_preview", {
          path,
          requestId: id,
        });
        if (!isCurrent(id)) return;
        setLoading({ path, phase: "upload" });
        let renderer = rendererRef.current;
        if (!renderer) {
          if (!canvasRef.current)
            throw new Error("The preview canvas is unavailable.");
          renderer = new SchematicRenderer(canvasRef.current, graphicsFailed);
          if (!isCurrent(id)) {
            renderer.dispose();
            return;
          }
          rendererRef.current = renderer;
          renderer.setGrid(gridRef.current);
        }
        const metadata = await renderer.load(buffer, () => isCurrent(id));
        if (!isCurrent(id)) return;
        setLoaded({
          path,
          metadata,
          seconds: (performance.now() - started) / 1000,
        });
        setLoading(null);
        updateTitle(path, id);
        // React has committed the active view before the native decode finishes.
        canvasRef.current?.focus({ preventScroll: true });
      } catch (error) {
        if (!isCurrent(id)) return;
        rendererRef.current?.clear();
        setLoading(null);
        if (errorMessage(error) !== "Cancelled")
          setNotice({ intent: "error", message: errorMessage(error) });
      }
    },
    [cancelNative, clearView, graphicsFailed, isCurrent, updateTitle],
  );

  const chooseFile = useCallback(async () => {
    if (!mounted.current || !bootstrapRef.current || choosingRef.current)
      return;
    choosingRef.current = true;
    setChoosing(true);
    // Dismissing the picker must leave the current preview or load intact.
    const id = generation.current;
    try {
      const path = await invoke<string | null>("choose_file");
      if (path && isCurrent(id)) void loadPath(path);
    } catch (error) {
      if (isCurrent(id))
        setNotice({ intent: "error", message: errorMessage(error) });
    } finally {
      choosingRef.current = false;
      if (mounted.current) setChoosing(false);
    }
  }, [isCurrent, loadPath]);

  const nativeAction = useCallback(
    async (
      command:
        "register_associations" | "unregister_associations" | "show_licenses",
    ) => {
      if (actionBusyRef.current) return;
      actionBusyRef.current = true;
      setActionBusy(true);
      const id = generation.current;
      try {
        await invoke(command);
        if (isCurrent(id)) {
          setNotice({
            intent: "success",
            message:
              command === "register_associations"
                ? "File associations registered. Choose Litematica Preview for your schematic files in Windows Settings."
                : command === "unregister_associations"
                  ? "File associations for this copy of Litematica Preview were removed."
                  : "The bundled licenses folder has been opened.",
          });
        }
      } catch (error) {
        if (isCurrent(id))
          setNotice({ intent: "error", message: errorMessage(error) });
      } finally {
        actionBusyRef.current = false;
        if (mounted.current) setActionBusy(false);
      }
    },
    [isCurrent],
  );

  useEffect(() => {
    mounted.current = true;
    // Keep restoration opted in even if a lost context makes the renderer
    // dispose itself before the browser dispatches webglcontextlost.
    const canvas = canvasRef.current;
    const allowContextRestore = (event: Event) => event.preventDefault();
    canvas?.addEventListener("webglcontextlost", allowContextRestore);
    let active = true;
    let unlisten: (() => void) | undefined;
    const initialGeneration = generation.current;
    void invoke<Bootstrap>("bootstrap")
      .then((result) => {
        if (!active) return;
        const openInitialPath = isCurrent(initialGeneration);
        generation.current = Math.max(generation.current, result.requestId);
        bootstrapRef.current = result;
        setBootstrap(result);
        if (!initialPathConsumed.current) {
          initialPathConsumed.current = true;
          if (result.initialPath && openInitialPath)
            void loadPath(result.initialPath);
        }
      })
      .catch((error: unknown) => {
        if (active)
          setNotice({
            intent: "error",
            message: `Could not initialize the application: ${errorMessage(error)}`,
          });
      });
    void getCurrentWebview()
      .onDragDropEvent(({ payload }) => {
        if (!active) return;
        if (payload.type === "leave") {
          setDragging(false);
        } else if (payload.type === "enter" || payload.type === "over") {
          setDragging(true);
        } else if (payload.type === "drop") {
          setDragging(false);
          const extensions = bootstrapRef.current?.extensions;
          if (!extensions) {
            setNotice({
              intent: "info",
              message:
                "The application is still starting. Please drop your file again in a moment.",
            });
            return;
          }
          const path = payload.paths.find((candidate) =>
            extensions.some((extension) =>
              candidate.toLowerCase().endsWith(extension.toLowerCase()),
            ),
          );
          if (path) void loadPath(path);
          else
            setNotice({
              intent: "error",
              message: `No supported schematic was dropped. Supported formats: ${extensions.join(", ")}.`,
            });
        }
      })
      .then((dispose) => {
        if (active) unlisten = dispose;
        else dispose();
      })
      .catch((error: unknown) => {
        if (active)
          setNotice({
            intent: "error",
            message: `Drag and drop is unavailable. You can still use Open. ${errorMessage(error)}`,
          });
      });
    return () => {
      active = false;
      mounted.current = false;
      const id = ++generation.current;
      cancelNative(id);
      unlisten?.();
      rendererRef.current?.dispose();
      rendererRef.current = null;
      canvas?.removeEventListener("webglcontextlost", allowContextRestore);
    };
  }, [cancelNative, isCurrent, loadPath]);

  useEffect(() => {
    const media = matchMedia("(prefers-color-scheme: dark)");
    const change = () => setSystemDark(media.matches);
    media.addEventListener("change", change);
    return () => media.removeEventListener("change", change);
  }, []);

  const dark = theme === "dark" || (theme === "system" && systemDark);
  useEffect(() => {
    document.documentElement.style.colorScheme = dark ? "dark" : "light";
    try {
      localStorage.setItem("litematica-preview-theme", theme);
    } catch {
      /* Theme still works when storage is disabled. */
    }
  }, [dark, theme]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.altKey) return;
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "o") {
        event.preventDefault();
        if (!dialog) void chooseFile();
        return;
      }
      if (dialog || event.ctrlKey || event.metaKey) return;
      const target = event.target;
      if (
        target instanceof HTMLElement &&
        (target.isContentEditable ||
          target.closest(
            'input, textarea, select, [role="menu"], [role="dialog"]',
          ))
      )
        return;
      if (event.key === "Escape" && loading) {
        event.preventDefault();
        home();
      } else if (
        event.key.toLowerCase() === "f" &&
        loaded &&
        target !== canvasRef.current
      ) {
        event.preventDefault();
        rendererRef.current?.fit();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [chooseFile, dialog, home, loaded, loading]);

  const toggleGrid = () => {
    const visible = !gridRef.current;
    gridRef.current = visible;
    setGrid(visible);
    rendererRef.current?.setGrid(visible);
  };
  const stageActive = Boolean(loaded || loading);
  const size = loaded?.metadata.max
    .map((maximum, axis) =>
      dimensions.format(maximum - loaded.metadata.min[axis]),
    )
    .join(" × ");

  return (
    <FluentProvider
      theme={dark ? webDarkTheme : webLightTheme}
      // Portals inherit theme tokens, not the full-window app-shell layout.
      applyStylesToPortals={false}
      className={`app-shell ${dark ? "theme-dark" : "theme-light"}`}
    >
      <header className="app-header">
        <div className="brand">
          <img src={icon} width="30" height="30" alt="" />
          <span>{appName}</span>
          <Badge appearance="outline" className="desktop-badge">
            Desktop
          </Badge>
        </div>
        <Menu
          checkedValues={{ theme: [theme] }}
          onCheckedValueChange={(_, data) => {
            const value = data.checkedItems[0];
            if (data.name === "theme" && isTheme(value)) setTheme(value);
          }}
        >
          <MenuTrigger disableButtonEnhancement>
            <Button
              appearance="subtle"
              icon={<MoreHorizontal20Regular />}
              aria-label="Application menu"
              title="Application menu"
            />
          </MenuTrigger>
          <MenuPopover>
            <MenuList>
              <MenuItem
                icon={<Keyboard20Regular />}
                onClick={() => setDialog("controls")}
              >
                Controls and shortcuts
              </MenuItem>
              <MenuItem
                icon={<Info20Regular />}
                onClick={() => setDialog("about")}
              >
                About and licenses
              </MenuItem>
              <MenuDivider />
              <MenuGroup>
                <MenuGroupHeader>File associations</MenuGroupHeader>
                <MenuItem
                  disabled={actionBusy}
                  onClick={() => void nativeAction("register_associations")}
                >
                  Set as default app…
                </MenuItem>
                <MenuItem
                  disabled={actionBusy}
                  onClick={() => void nativeAction("unregister_associations")}
                >
                  Remove file associations
                </MenuItem>
              </MenuGroup>
              <MenuDivider />
              <MenuGroup>
                <MenuGroupHeader>Appearance</MenuGroupHeader>
                <MenuItemRadio name="theme" value="system">
                  Use system setting
                </MenuItemRadio>
                <MenuItemRadio name="theme" value="light">
                  Light
                </MenuItemRadio>
                <MenuItemRadio name="theme" value="dark">
                  Dark
                </MenuItemRadio>
              </MenuGroup>
            </MenuList>
          </MenuPopover>
        </Menu>
      </header>

      <nav className="command-bar" aria-label="Preview commands">
        <Button
          appearance="primary"
          icon={<FolderOpen20Regular />}
          disabled={!bootstrap || choosing}
          onClick={() => void chooseFile()}
          title="Open schematic (Ctrl+O)"
        >
          Open<span className="shortcut-hint">Ctrl+O</span>
        </Button>
        <Tooltip content="Return home" relationship="label">
          <Button
            appearance="subtle"
            icon={<Home20Regular />}
            onClick={home}
            aria-label="Home"
          />
        </Tooltip>
        <span className="command-separator" />
        <Button
          appearance="subtle"
          icon={<ArrowExpand20Regular />}
          disabled={!loaded}
          onClick={() => rendererRef.current?.fit()}
          title="Fit schematic (F)"
        >
          Fit<span className="shortcut-hint">F</span>
        </Button>
        <Tooltip content="Zoom out (−)" relationship="label">
          <Button
            appearance="subtle"
            icon={<Subtract20Regular />}
            disabled={!loaded}
            onClick={() => rendererRef.current?.zoom(1.12)}
            aria-label="Zoom out"
          />
        </Tooltip>
        <Tooltip content="Zoom in (+)" relationship="label">
          <Button
            appearance="subtle"
            icon={<Add20Regular />}
            disabled={!loaded}
            onClick={() => rendererRef.current?.zoom(0.88)}
            aria-label="Zoom in"
          />
        </Tooltip>
        <span className="command-separator" />
        <Tooltip
          content={grid ? "Hide ground grid" : "Show ground grid"}
          relationship="label"
        >
          <ToggleButton
            appearance="subtle"
            icon={<Grid20Regular />}
            checked={grid}
            onClick={toggleGrid}
            aria-label="Ground grid"
          />
        </Tooltip>
        <span className="command-file" title={loaded?.path || loading?.path}>
          {loaded
            ? fileName(loaded.path)
            : loading
              ? fileName(loading.path)
              : "Ready when you are"}
        </span>
      </nav>

      {notice && (
        <MessageBar intent={notice.intent} className="app-notice">
          <MessageBarBody>
            <MessageBarTitle>
              {notice.intent === "error"
                ? "Something went wrong"
                : notice.intent === "success"
                  ? "Done"
                  : "Please note"}
            </MessageBarTitle>
            {notice.message}
          </MessageBarBody>
          <MessageBarActions
            containerAction={
              <Button
                appearance="transparent"
                icon={<Dismiss20Regular />}
                aria-label="Dismiss message"
                onClick={() => setNotice(null)}
              />
            }
          />
        </MessageBar>
      )}

      <main
        className="workspace"
        aria-label={stageActive ? "Schematic preview" : "Welcome"}
      >
        <section className="welcome" hidden={stageActive}>
          <div className="welcome-content">
            <div className="welcome-heading">
              <div className="hero-symbol">
                <Cube24Regular />
              </div>
              <span className="eyebrow">A CLOSER LOOK AT YOUR BUILDS</span>
            </div>
            <h1>
              Your next build,
              <br />
              from every angle.
            </h1>
            <p className="welcome-description">
              Open Minecraft schematics and structures in a focused, interactive
              3D preview. No game launch needed.
            </p>
            <div className="open-panel">
              <div className="open-panel-icon">
                <FolderOpen20Regular />
              </div>
              <div className="open-panel-copy">
                <strong>Drop a schematic here</strong>
                <span>or choose a file from your computer</span>
              </div>
              <Button
                appearance="primary"
                icon={<FolderOpen20Regular />}
                disabled={!bootstrap || choosing}
                onClick={() => void chooseFile()}
              >
                {choosing ? "Choosing file…" : "Open schematic"}
              </Button>
            </div>
            <div className="format-list" aria-label="Supported formats">
              {bootstrap ? (
                bootstrap.extensions.map((extension) => (
                  <Badge key={extension} appearance="outline" shape="rounded">
                    {extension}
                  </Badge>
                ))
              ) : (
                <span className="muted" role="status">
                  Preparing the schematic viewer…
                </span>
              )}
            </div>
            {bootstrap && bootstrap.demos.length > 0 && (
              <section className="demo-section" aria-labelledby="demos-heading">
                <div className="section-heading">
                  <h2 id="demos-heading">Try a bundled example</h2>
                  <span>Explore a format, no download required</span>
                </div>
                <div className="demo-grid">
                  {bootstrap.demos.map((demo) => (
                    <Button
                      key={demo.path}
                      appearance="outline"
                      className="demo-card"
                      onClick={() => void loadPath(demo.path)}
                      aria-label={`Open ${demo.name}, ${demo.extension}`}
                    >
                      <span className="demo-symbol">
                        <Document20Regular />
                      </span>
                      <span className="demo-copy">
                        <strong>{demo.name}</strong>
                        <span>{demo.extension}</span>
                      </span>
                      <ArrowRight20Regular className="demo-arrow" />
                    </Button>
                  ))}
                </div>
              </section>
            )}
            <p className="privacy-note">
              <ShieldCheckmark20Regular />
              <span>
                Your schematics stay on this PC. Decoding and rendering work
                entirely offline.
              </span>
            </p>
          </div>
        </section>

        <section
          className={`preview-stage${stageActive ? " is-active" : ""}`}
          aria-label="Interactive 3D model"
          aria-hidden={!stageActive}
          aria-busy={Boolean(loading)}
        >
          <canvas
            ref={canvasRef}
            className="preview-canvas"
            tabIndex={loaded ? 0 : -1}
            aria-label={
              loaded ? `3D preview of ${fileName(loaded.path)}` : "3D preview"
            }
            aria-describedby="canvas-controls"
          />
          <p id="canvas-controls" className="sr-only">
            Drag to orbit. Right or middle drag to pan. Scroll to zoom. Arrow
            keys orbit; Shift and arrow keys pan. Plus and minus zoom. F or Home
            fits the model.
          </p>
          {loaded && (
            <div className="canvas-hint" aria-hidden="true">
              Drag to orbit<span>·</span>Right-drag to pan<span>·</span>Scroll
              to zoom
            </div>
          )}
          {loading && (
            <div className="loading-overlay">
              <div className="loading-card" role="status" aria-live="polite">
                <div className="loading-symbol">
                  <Cube24Regular />
                </div>
                <h2>
                  {loading.phase === "decode"
                    ? "Preparing your schematic"
                    : "Building the preview"}
                </h2>
                <p className="loading-filename" title={loading.path}>
                  {fileName(loading.path)}
                </p>
                <p className="muted">
                  {loading.phase === "decode"
                    ? "Reading blocks and generating geometry locally."
                    : "Uploading geometry and textures to your graphics device."}
                </p>
                <Button appearance="secondary" onClick={home}>
                  Cancel<span className="shortcut-hint">Esc</span>
                </Button>
              </div>
            </div>
          )}
        </section>

        {dragging && (
          <div className="drop-overlay" role="status">
            <div>
              <FolderOpen20Regular />
              <h2>Drop to preview</h2>
              <p>The first supported schematic will open.</p>
            </div>
          </div>
        )}
      </main>

      <footer className="status-bar" aria-label="Preview information">
        {loaded ? (
          <>
            <span>
              <strong>{numbers.format(loaded.metadata.blockCount)}</strong>{" "}
              blocks
            </span>
            <span title="Geometry dimensions in blocks">{size} blocks</span>
            <span>
              {numbers.format(loaded.metadata.triangleCount)} triangles
            </span>
            <span className="load-time">
              Loaded in {loaded.seconds.toFixed(2)} s
            </span>
          </>
        ) : (
          <>
            <span role="status">
              {loading
                ? loading.phase === "decode"
                  ? "Decoding and meshing…"
                  : "Uploading to graphics device…"
                : choosing
                  ? "Choose a schematic in the file dialog"
                  : "Local files. A clearer view."}
            </span>
            <span className="status-offline">Works offline</span>
          </>
        )}
      </footer>

      <Dialog
        open={dialog !== null}
        onOpenChange={(_, data) => {
          if (!data.open) setDialog(null);
        }}
      >
        <DialogSurface>
          <DialogBody>
            <DialogTitle>
              {dialog === "controls"
                ? "Controls and shortcuts"
                : `About ${appName}`}
            </DialogTitle>
            <DialogContent>
              {dialog === "controls" ? (
                <>
                  <p className="dialog-intro">
                    Click or Tab into the preview to use its keyboard controls.
                  </p>
                  <dl className="controls-list">
                    <div>
                      <dt>Orbit</dt>
                      <dd>Left-drag / Arrow keys</dd>
                    </div>
                    <div>
                      <dt>Pan</dt>
                      <dd>Right- or middle-drag / Shift + Arrow keys</dd>
                    </div>
                    <div>
                      <dt>Zoom</dt>
                      <dd>
                        Scroll / <kbd>+</kbd> or <kbd>−</kbd>
                      </dd>
                    </div>
                    <div>
                      <dt>Fit model</dt>
                      <dd>
                        <kbd>F</kbd> / <kbd>Home</kbd> in the preview
                      </dd>
                    </div>
                    <div>
                      <dt>Open schematic</dt>
                      <dd>
                        <kbd>Ctrl</kbd> + <kbd>O</kbd>
                      </dd>
                    </div>
                    <div>
                      <dt>Cancel loading</dt>
                      <dd>
                        <kbd>Esc</kbd>
                      </dd>
                    </div>
                  </dl>
                  <p className="muted">
                    Use the grid button to show or hide the ground grid. Home on
                    the command bar returns to your bundled examples.
                  </p>
                </>
              ) : (
                <div className="about-content">
                  <div className="about-brand">
                    <img src={icon} width="48" height="48" alt="" />
                    <div>
                      <strong>{appName}</strong>
                      <span>Version {bootstrap?.version || "unavailable"}</span>
                    </div>
                  </div>
                  <p>
                    A local, interactive viewer for Minecraft schematics and
                    structures, inspired by LitematicaQL.
                  </p>
                  <p>
                    Powered by Nucleation, Tauri, WebGL, React, and Fluent UI.
                  </p>
                  <p>
                    Distributed under the GNU Affero General Public License v3.
                    This program comes with no warranty. Redistribution is
                    permitted under the terms of the bundled license.
                  </p>
                  <p className="muted">
                    The application license and third-party notices are included
                    with your installation.
                  </p>
                </div>
              )}
            </DialogContent>
            <DialogActions>
              {dialog === "about" && (
                <Button
                  disabled={actionBusy}
                  onClick={() => {
                    setDialog(null);
                    void nativeAction("show_licenses");
                  }}
                >
                  Open licenses folder
                </Button>
              )}
              <Button appearance="primary" onClick={() => setDialog(null)}>
                Close
              </Button>
            </DialogActions>
          </DialogBody>
        </DialogSurface>
      </Dialog>
    </FluentProvider>
  );
}
