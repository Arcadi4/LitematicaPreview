import { mat4, vec3 } from "gl-matrix";

export type PreviewMetadata = {
  blockCount: number;
  blockEntityCount: number;
  triangleCount: number;
  min: [number, number, number];
  max: [number, number, number];
  textures: { width: number; height: number; byteLength: number }[];
  parts: {
    vertexCount: number;
    indexCount: number;
    textureIndex: number;
    alphaMode: 0 | 1 | 2;
  }[];
};

type Part = {
  vao: WebGLVertexArrayObject;
  buffers: WebGLBuffer[];
  texture: WebGLTexture;
  count: number;
  alphaMode: 0 | 1 | 2;
};

type Model = {
  textures: WebGLTexture[];
  parts: Part[];
  gridVao: WebGLVertexArrayObject | null;
  gridBuffer: WebGLBuffer | null;
  gridCount: number;
  released: boolean;
};

type Pipeline = {
  scene: WebGLProgram;
  output: WebGLProgram;
  fullscreen: WebGLVertexArrayObject;
  matrix: WebGLUniformLocation;
  offset: WebGLUniformLocation;
  alphaMode: WebGLUniformLocation;
  grid: WebGLUniformLocation;
};

type Targets = {
  width: number;
  height: number;
  color: WebGLTexture | null;
  resolve: WebGLFramebuffer | null;
  draw: WebGLFramebuffer | null;
  depth: WebGLRenderbuffer | null;
  multisampleColor: WebGLRenderbuffer | null;
};

type UploadBudget = { bytes: number; started: number };

const FOV = (28 * Math.PI) / 180;
const HALF_FOV_TAN = Math.tan(FOV / 2);
const ATTRIBUTE_SIZES = [3, 3, 2, 4] as const;
const UPLOAD_CHUNK = 1024 * 1024;
const MAX_METADATA_BYTES = 16 * 1024 * 1024;
const MAX_RENDER_PIXELS = 16 * 1024 * 1024;
const UP = new Float32Array([0, 1, 0]);

function required<T>(value: T | null, name: string): T {
  if (value === null)
    throw new Error(
      `The graphics device could not allocate ${name}. It may be out of graphics memory.`,
    );
  return value;
}

function errorOf(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}

function integer(
  value: unknown,
  name: string,
  maximum = Number.MAX_SAFE_INTEGER,
  minimum = 0,
): number {
  if (
    typeof value !== "number" ||
    !Number.isSafeInteger(value) ||
    value < minimum ||
    value > maximum
  ) {
    throw new Error(`Invalid preview ${name}.`);
  }
  return value;
}

function record(value: unknown, name: string): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value))
    throw new Error(`Invalid preview ${name}.`);
  return value as Record<string, unknown>;
}

// Only the small metadata is decoded. All pixel/vertex/index payloads stay in
// the original binary buffer until their synchronous WebGL upload completes.
function parsePreview(
  buffer: ArrayBuffer,
  maxTextureSize: number,
): { metadata: PreviewMetadata; offset: number } {
  if (buffer.byteLength < 8 || buffer.byteLength % 4 !== 0)
    throw new Error("The preview binary is truncated.");
  const header = new DataView(buffer, 0, 8);
  if (header.getUint32(0, true) !== 0x3156504c)
    throw new Error("The preview binary has an unsupported version.");
  const length = header.getUint32(4, true);
  if (
    length === 0 ||
    length > MAX_METADATA_BYTES ||
    length > buffer.byteLength - 8
  ) {
    throw new Error("The preview metadata length is invalid.");
  }
  const metadata: unknown = JSON.parse(
    new TextDecoder("utf-8", { fatal: true }).decode(
      new Uint8Array(buffer, 8, length),
    ),
  );
  const root = record(metadata, "metadata");
  integer(root.blockCount, "block count", Number.MAX_SAFE_INTEGER, 1);
  integer(root.blockEntityCount, "block entity count");
  integer(root.triangleCount, "triangle count", Number.MAX_SAFE_INTEGER, 1);
  for (const bound of [root.min, root.max]) {
    if (
      !Array.isArray(bound) ||
      bound.length !== 3 ||
      bound.some((v: unknown) => typeof v !== "number" || !Number.isFinite(v))
    ) {
      throw new Error("The preview geometry bounds are invalid.");
    }
  }
  const min = root.min as number[];
  const max = root.max as number[];
  for (let axis = 0; axis < 3; axis++) {
    if (min[axis] > max[axis])
      throw new Error("The preview geometry bounds are reversed.");
  }
  if (
    !Array.isArray(root.textures) ||
    root.textures.length === 0 ||
    root.textures.length > length / 2 ||
    !Array.isArray(root.parts) ||
    root.parts.length === 0 ||
    root.parts.length > length / 2
  ) {
    throw new Error(
      "The preview must contain textures and renderable mesh parts.",
    );
  }
  const offset = Math.ceil((8 + length) / 4) * 4;
  let end = offset;
  const consume = (bytes: number) => {
    if (
      !Number.isSafeInteger(bytes) ||
      bytes < 0 ||
      bytes > buffer.byteLength - end
    ) {
      throw new Error(
        "The preview binary contains truncated or oversized geometry.",
      );
    }
    end += bytes;
  };
  for (const value of root.textures) {
    const texture = record(value, "texture");
    const width = integer(texture.width, "texture width", 0x7fffffff, 1);
    const height = integer(texture.height, "texture height", 0x7fffffff, 1);
    if (width > maxTextureSize || height > maxTextureSize) {
      throw new Error(
        `A block texture exceeds the graphics device's ${maxTextureSize}-pixel texture limit.`,
      );
    }
    const bytes = integer(texture.byteLength, "texture byte length");
    if (bytes !== width * height * 4)
      throw new Error("A block texture has an invalid RGBA byte length.");
    consume(bytes);
  }
  let triangles = 0;
  for (const value of root.parts) {
    const part = record(value, "mesh part");
    const vertices = integer(part.vertexCount, "vertex count", 0xffffffff, 1);
    const indices = integer(part.indexCount, "index count", 0x7fffffff, 1);
    integer(part.textureIndex, "texture index", root.textures.length - 1);
    integer(part.alphaMode, "alpha mode", 2);
    if (indices % 3 !== 0)
      throw new Error("A preview mesh contains an incomplete triangle.");
    consume(vertices * 12 * 4 + indices * 4);
    triangles += indices / 3;
  }
  if (triangles !== root.triangleCount)
    throw new Error(
      "The preview triangle count does not match its mesh parts.",
    );
  if (end !== buffer.byteLength)
    throw new Error("The preview binary has an unexpected payload length.");
  return { metadata: metadata as PreviewMetadata, offset };
}

const VERTEX_SOURCE = `#version 300 es
precision highp float;
layout(location=0) in vec3 position;
layout(location=1) in vec3 normal;
layout(location=2) in vec2 uv;
layout(location=3) in vec4 color;
uniform mat4 mvp;
uniform vec3 offset;
out vec3 surfaceNormal;
out vec2 textureUv;
out vec4 tint;
void main() {
  gl_Position = mvp * vec4(position + offset, 1.0);
  surfaceNormal = normal;
  textureUv = uv;
  tint = color;
}`;

const FRAGMENT_SOURCE = `#version 300 es
precision highp float;
in vec3 surfaceNormal;
in vec2 textureUv;
in vec4 tint;
uniform sampler2D blockTexture;
uniform int alphaMode;
uniform bool isGrid;
out vec4 pixel;
void main() {
  if (isGrid) {
    pixel = vec4(0.020, 0.030, 0.042, 1.0);
    return;
  }
  // SRGB8_ALPHA8 textures are decoded by the sampler. Nucleation's tint/AO
  // multiplier and lighting are applied in linear space, as in the native view.
  vec4 base = texture(blockTexture, textureUv) * tint;
  if (alphaMode == 1 && base.a < 0.5) discard;
  vec3 n = normalize(surfaceNormal) * (gl_FrontFacing ? 1.0 : -1.0);
  float light = 0.68 + 0.32 * max(dot(n, normalize(vec3(1.0, 1.4, 0.8))), 0.0);
  pixel = vec4(base.rgb * light, alphaMode == 2 ? base.a : 1.0);
}`;

const OUTPUT_VERTEX_SOURCE = `#version 300 es
precision highp float;
out vec2 textureUv;
void main() {
  vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
  textureUv = p;
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`;

const OUTPUT_FRAGMENT_SOURCE = `#version 300 es
precision highp float;
in vec2 textureUv;
uniform sampler2D linearFrame;
out vec4 pixel;
void main() {
  vec3 linear = texture(linearFrame, textureUv).rgb;
  vec3 srgb = mix(linear * 12.92, 1.055 * pow(linear, vec3(1.0 / 2.4)) - 0.055,
                  greaterThan(linear, vec3(0.0031308)));
  pixel = vec4(srgb, 1.0);
}`;

function compile(
  gl: WebGL2RenderingContext,
  type: number,
  source: string,
): WebGLShader {
  const shader = required(gl.createShader(type), "a shader");
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (gl.getShaderParameter(shader, gl.COMPILE_STATUS)) return shader;
  const message =
    gl.getShaderInfoLog(shader) || "Unknown shader compilation error.";
  gl.deleteShader(shader);
  throw new Error(`The WebGL 2 shader could not compile: ${message}`);
}

function program(
  gl: WebGL2RenderingContext,
  vertexSource: string,
  fragmentSource: string,
): WebGLProgram {
  const vertex = compile(gl, gl.VERTEX_SHADER, vertexSource);
  let fragment: WebGLShader | null = null;
  let linked: WebGLProgram | null = null;
  try {
    fragment = compile(gl, gl.FRAGMENT_SHADER, fragmentSource);
    linked = required(gl.createProgram(), "a shader program");
    gl.attachShader(linked, vertex);
    gl.attachShader(linked, fragment);
    gl.linkProgram(linked);
    if (!gl.getProgramParameter(linked, gl.LINK_STATUS)) {
      throw new Error(
        `The WebGL 2 program could not link: ${gl.getProgramInfoLog(linked) || "Unknown linking error."}`,
      );
    }
    gl.detachShader(linked, vertex);
    gl.detachShader(linked, fragment);
    return linked;
  } catch (error) {
    if (linked) gl.deleteProgram(linked);
    throw error;
  } finally {
    gl.deleteShader(vertex);
    if (fragment) gl.deleteShader(fragment);
  }
}

export class SchematicRenderer {
  private readonly gl: WebGL2RenderingContext;
  private pipeline: Pipeline | null = null;
  private targets: Targets | null = null;
  private model: Model | null = null;
  private readonly staged = new Set<Model>();
  private readonly yieldChannel = new MessageChannel();
  private readonly pendingYields: (() => void)[] = [];
  private readonly observer: ResizeObserver;
  private dprQuery: MediaQueryList | null = null;
  private generation = 0;
  private frame = 0;
  private disposed = false;
  private contextLost = false;
  private failed = false;
  private gridVisible = true;
  private maxTextureSize = 1;
  private maxWidth = 1;
  private maxHeight = 1;
  private samples = 1;
  private aspect = 1;
  private cssHeight = 1;
  private yaw = Math.PI / 4;
  private pitch = Math.atan(1 / Math.sqrt(2));
  private distance = 10;
  private fittedDistance = 10;
  private readonly centre = new Float32Array(3);
  private readonly minimum = new Float32Array(3);
  private readonly maximum = new Float32Array(3);
  private readonly target = new Float32Array(3);
  private readonly direction = new Float32Array(3);
  private readonly forward = new Float32Array(3);
  private readonly right = new Float32Array(3);
  private readonly up = new Float32Array(3);
  private readonly eye = new Float32Array(3);
  private readonly view = new Float32Array(16);
  private readonly projection = new Float32Array(16);
  private readonly mvp = new Float32Array(16);
  private pointerId: number | null = null;
  private pointerButton = 0;
  private lastX = 0;
  private lastY = 0;
  private readonly originalTabIndex: string | null;
  private readonly originalTouchAction: string;

  constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly onError: (message: string) => void,
  ) {
    this.originalTabIndex = canvas.getAttribute("tabindex");
    this.originalTouchAction = canvas.style.touchAction;
    this.observer = new ResizeObserver(this.onResize);
    const gl = canvas.getContext("webgl2", {
      alpha: false,
      antialias: false,
      depth: false,
      stencil: false,
      premultipliedAlpha: false,
      preserveDrawingBuffer: false,
    });
    if (!gl) {
      this.observer.disconnect();
      this.yieldChannel.port1.close();
      this.yieldChannel.port2.close();
      const message =
        "WebGL 2 is unavailable. Enable graphics acceleration or update your graphics driver to preview schematics.";
      onError(message);
      throw new Error(message);
    }
    this.gl = gl;
    try {
      this.initialize();
    } catch (error) {
      this.observer.disconnect();
      this.yieldChannel.port1.close();
      this.yieldChannel.port2.close();
      onError(errorOf(error).message);
      throw error;
    }
    this.yieldChannel.port1.onmessage = () => this.pendingYields.shift()?.();
    if (canvas.tabIndex < 0) canvas.tabIndex = 0;
    canvas.style.touchAction = "none";
    canvas.addEventListener("pointerdown", this.onPointerDown);
    canvas.addEventListener("pointermove", this.onPointerMove);
    canvas.addEventListener("pointerup", this.onPointerEnd);
    canvas.addEventListener("pointercancel", this.onPointerEnd);
    canvas.addEventListener("lostpointercapture", this.onPointerEnd);
    canvas.addEventListener("wheel", this.onWheel, { passive: false });
    canvas.addEventListener("keydown", this.onKeyDown);
    canvas.addEventListener("contextmenu", this.onContextMenu);
    canvas.addEventListener("webglcontextlost", this.onContextLost);
    canvas.addEventListener("webglcontextrestored", this.onContextRestored);
    window.addEventListener("resize", this.onResize);
    this.observer.observe(canvas);
    this.watchDpr();
    this.syncSize();
    this.invalidate();
  }

  async load(
    buffer: ArrayBuffer,
    isCurrent: () => boolean,
  ): Promise<PreviewMetadata> {
    if (this.disposed || !isCurrent()) throw new Error("Cancelled");
    const generation = ++this.generation;
    this.cancelStaged();
    const model: Model = {
      textures: [],
      parts: [],
      gridVao: null,
      gridBuffer: null,
      gridCount: 0,
      released: false,
    };
    const guard = () => {
      if (this.disposed || generation !== this.generation || !isCurrent())
        throw new Error("Cancelled");
      if (this.contextLost || this.gl.isContextLost())
        throw new Error(
          "The graphics context was lost. Reopen the schematic after the graphics device recovers.",
        );
      if (!this.pipeline)
        throw new Error(
          "The graphics renderer is unavailable. Reopen the application to initialize the graphics device.",
        );
    };
    this.staged.add(model);
    const gl = this.gl;
    const budget: UploadBudget = { bytes: 0, started: performance.now() };
    try {
      guard();
      const parsed = parsePreview(buffer, this.maxTextureSize);
      const metadata = parsed.metadata;
      let offset = parsed.offset;
      for (let i = 0; i < metadata.textures.length; i++) {
        guard();
        const source = metadata.textures[i];
        const texture = required(gl.createTexture(), "a block texture");
        model.textures.push(texture);
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, texture);
        gl.texStorage2D(
          gl.TEXTURE_2D,
          1,
          gl.SRGB8_ALPHA8,
          source.width,
          source.height,
        );
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
        const wrap = i === 0 ? gl.CLAMP_TO_EDGE : gl.REPEAT;
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, wrap);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, wrap);
        gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
        gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
        gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
        gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.NONE);
        this.checkGraphics("allocate a block texture");
        await this.checkpoint(budget, 0, guard);
        guard();
        const rowBytes = source.width * 4;
        const rowsPerChunk = Math.max(1, Math.floor(UPLOAD_CHUNK / rowBytes));
        for (let row = 0; row < source.height; row += rowsPerChunk) {
          guard();
          const rows = Math.min(rowsPerChunk, source.height - row);
          const pixels = new Uint8Array(
            buffer,
            offset + row * rowBytes,
            rows * rowBytes,
          );
          gl.activeTexture(gl.TEXTURE0);
          gl.bindTexture(gl.TEXTURE_2D, texture);
          gl.texSubImage2D(
            gl.TEXTURE_2D,
            0,
            0,
            row,
            source.width,
            rows,
            gl.RGBA,
            gl.UNSIGNED_BYTE,
            pixels,
          );
          await this.checkpoint(budget, pixels.byteLength, guard);
          guard();
        }
        this.checkGraphics("upload a block texture");
        offset += source.byteLength;
      }
      for (const source of metadata.parts) {
        guard();
        const part: Part = {
          vao: required(gl.createVertexArray(), "a mesh vertex array"),
          buffers: [],
          texture: model.textures[source.textureIndex],
          count: source.indexCount,
          alphaMode: source.alphaMode,
        };
        model.parts.push(part);
        for (
          let attribute = 0;
          attribute < ATTRIBUTE_SIZES.length;
          attribute++
        ) {
          const size = ATTRIBUTE_SIZES[attribute];
          const values = new Float32Array(
            buffer,
            offset,
            source.vertexCount * size,
          );
          const gpu = required(gl.createBuffer(), "a mesh attribute buffer");
          part.buffers.push(gpu);
          await this.uploadBuffer(
            part.vao,
            gpu,
            gl.ARRAY_BUFFER,
            values,
            budget,
            guard,
          );
          guard();
          gl.bindVertexArray(part.vao);
          gl.bindBuffer(gl.ARRAY_BUFFER, gpu);
          gl.enableVertexAttribArray(attribute);
          gl.vertexAttribPointer(attribute, size, gl.FLOAT, false, 0, 0);
          offset += values.byteLength;
        }
        const indices = new Uint32Array(buffer, offset, source.indexCount);
        const gpu = required(gl.createBuffer(), "a mesh index buffer");
        part.buffers.push(gpu);
        await this.uploadBuffer(
          part.vao,
          gpu,
          gl.ELEMENT_ARRAY_BUFFER,
          indices,
          budget,
          guard,
        );
        guard();
        offset += indices.byteLength;
      }
      guard();
      this.buildGrid(model, metadata);
      this.checkGraphics("upload the schematic");
      guard();
      gl.bindVertexArray(null);
      this.staged.delete(model);
      const previous = this.model;
      this.model = model;
      for (let axis = 0; axis < 3; axis++) {
        this.centre[axis] = (metadata.min[axis] + metadata.max[axis]) * 0.5;
        this.minimum[axis] = metadata.min[axis] - this.centre[axis];
        this.maximum[axis] = metadata.max[axis] - this.centre[axis];
      }
      this.releaseModel(previous);
      this.failed = false;
      this.syncSize();
      this.fit();
      return metadata;
    } catch (error) {
      this.staged.delete(model);
      this.releaseModel(model);
      if (this.disposed || generation !== this.generation || !isCurrent())
        throw new Error("Cancelled");
      const failure = errorOf(error);
      if (failure.message !== "Cancelled") this.onError(failure.message);
      throw failure;
    }
  }

  clear(): void {
    if (this.disposed) return;
    this.generation++;
    this.cancelStaged();
    this.releaseModel(this.model);
    this.model = null;
    this.failed = false;
    this.invalidate();
  }

  fit(): void {
    if (!this.model || this.disposed) return;
    this.yaw = Math.PI / 4;
    this.pitch = Math.atan(1 / Math.sqrt(2));
    this.target.fill(0);
    this.fittedDistance = this.distance = this.fittingDistance();
    this.invalidate();
  }

  zoom(factor: number): void {
    if (!this.model || this.disposed || !Number.isFinite(factor) || factor <= 0)
      return;
    this.distance = Math.min(
      this.fittedDistance * 8,
      Math.max(this.fittedDistance * 0.05, this.distance * factor),
    );
    this.invalidate();
  }

  setGrid(visible: boolean): void {
    if (this.gridVisible === visible || this.disposed) return;
    this.gridVisible = visible;
    this.invalidate();
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.generation++;
    this.cancelFrame();
    this.endPointer();
    this.observer.disconnect();
    this.dprQuery?.removeEventListener("change", this.onDprChange);
    window.removeEventListener("resize", this.onResize);
    const canvas = this.canvas;
    canvas.removeEventListener("pointerdown", this.onPointerDown);
    canvas.removeEventListener("pointermove", this.onPointerMove);
    canvas.removeEventListener("pointerup", this.onPointerEnd);
    canvas.removeEventListener("pointercancel", this.onPointerEnd);
    canvas.removeEventListener("lostpointercapture", this.onPointerEnd);
    canvas.removeEventListener("wheel", this.onWheel);
    canvas.removeEventListener("keydown", this.onKeyDown);
    canvas.removeEventListener("contextmenu", this.onContextMenu);
    canvas.removeEventListener("webglcontextlost", this.onContextLost);
    canvas.removeEventListener("webglcontextrestored", this.onContextRestored);
    canvas.style.touchAction = this.originalTouchAction;
    if (this.originalTabIndex === null) canvas.removeAttribute("tabindex");
    else canvas.setAttribute("tabindex", this.originalTabIndex);
    this.cancelStaged();
    this.releaseModel(this.model);
    this.model = null;
    this.releaseTargets(this.targets);
    this.targets = null;
    this.releasePipeline();
    this.yieldChannel.port1.onmessage = null;
    this.yieldChannel.port1.close();
    this.yieldChannel.port2.close();
  }

  private initialize(): void {
    const gl = this.gl;
    let scene: WebGLProgram | null = null;
    let output: WebGLProgram | null = null;
    let fullscreen: WebGLVertexArrayObject | null = null;
    try {
      scene = program(gl, VERTEX_SOURCE, FRAGMENT_SOURCE);
      output = program(gl, OUTPUT_VERTEX_SOURCE, OUTPUT_FRAGMENT_SOURCE);
      fullscreen = required(gl.createVertexArray(), "the output vertex array");
      const uniform = (name: string) => {
        const location = gl.getUniformLocation(scene!, name);
        if (location === null)
          throw new Error(
            `The graphics shader is missing its ${name} uniform.`,
          );
        return location;
      };
      const pipeline: Pipeline = {
        scene,
        output,
        fullscreen,
        matrix: uniform("mvp"),
        offset: uniform("offset"),
        alphaMode: uniform("alphaMode"),
        grid: uniform("isGrid"),
      };
      gl.useProgram(scene);
      gl.uniform1i(gl.getUniformLocation(scene, "blockTexture"), 0);
      gl.useProgram(output);
      gl.uniform1i(gl.getUniformLocation(output, "linearFrame"), 0);
      gl.frontFace(gl.CCW);
      gl.cullFace(gl.BACK);
      gl.depthFunc(gl.LESS);
      this.maxTextureSize = gl.getParameter(gl.MAX_TEXTURE_SIZE) as number;
      const renderSize = gl.getParameter(gl.MAX_RENDERBUFFER_SIZE) as number;
      const viewport = gl.getParameter(gl.MAX_VIEWPORT_DIMS) as Int32Array;
      this.maxWidth = Math.min(this.maxTextureSize, renderSize, viewport[0]);
      this.maxHeight = Math.min(this.maxTextureSize, renderSize, viewport[1]);
      const colorSamples = gl.getInternalformatParameter(
        gl.RENDERBUFFER,
        gl.SRGB8_ALPHA8,
        gl.SAMPLES,
      ) as Int32Array;
      const depthSamples = gl.getInternalformatParameter(
        gl.RENDERBUFFER,
        gl.DEPTH_COMPONENT24,
        gl.SAMPLES,
      ) as Int32Array;
      this.samples = 1;
      for (const count of colorSamples) {
        if (count <= 4 && count > this.samples && depthSamples.includes(count))
          this.samples = count;
      }
      this.checkGraphics("initialize WebGL 2");
      this.pipeline = pipeline;
    } catch (error) {
      if (scene) gl.deleteProgram(scene);
      if (output) gl.deleteProgram(output);
      if (fullscreen) gl.deleteVertexArray(fullscreen);
      throw error;
    }
  }

  private async uploadBuffer(
    vao: WebGLVertexArrayObject,
    gpu: WebGLBuffer,
    target: number,
    values: Float32Array<ArrayBuffer> | Uint32Array<ArrayBuffer>,
    budget: UploadBudget,
    guard: () => void,
  ): Promise<void> {
    guard();
    const gl = this.gl;
    gl.bindVertexArray(vao);
    gl.bindBuffer(target, gpu);
    gl.bufferData(target, values.byteLength, gl.STATIC_DRAW);
    this.checkGraphics("allocate a mesh buffer");
    await this.checkpoint(budget, 0, guard);
    guard();
    const elementsPerChunk = UPLOAD_CHUNK / 4;
    for (
      let element = 0;
      element < values.length;
      element += elementsPerChunk
    ) {
      guard();
      const count = Math.min(elementsPerChunk, values.length - element);
      // A frame or another load may have rebound every GL target while yielding.
      gl.bindVertexArray(vao);
      gl.bindBuffer(target, gpu);
      gl.bufferSubData(target, element * 4, values, element, count);
      await this.checkpoint(budget, count * 4, guard);
      guard();
    }
    this.checkGraphics("upload a mesh buffer");
  }

  private async checkpoint(
    budget: UploadBudget,
    bytes: number,
    guard: () => void,
  ): Promise<void> {
    guard();
    budget.bytes += bytes;
    if (
      budget.bytes < 4 * UPLOAD_CHUNK &&
      performance.now() - budget.started < 6
    )
      return;
    this.checkGraphics("upload the schematic");
    this.gl.bindVertexArray(null);
    await new Promise<void>((resolve) => {
      this.pendingYields.push(resolve);
      this.yieldChannel.port2.postMessage(null);
    });
    guard();
    budget.bytes = 0;
    budget.started = performance.now();
  }

  private buildGrid(model: Model, metadata: PreviewMetadata): void {
    const extent = Math.max(
      Math.max(
        metadata.max[0] - metadata.min[0],
        metadata.max[2] - metadata.min[2],
      ) * 1.15,
      16,
    );
    const step = Math.max(1, Math.ceil(extent / 128));
    const halfSteps = Math.ceil(extent / (2 * step));
    const edge = halfSteps * step;
    const y = (metadata.min[1] - metadata.max[1]) * 0.5 - 0.02;
    const lines = new Float32Array((halfSteps * 2 + 1) * 12);
    let at = 0;
    for (let line = -halfSteps; line <= halfSteps; line++) {
      const offset = line * step;
      lines[at++] = offset;
      lines[at++] = y;
      lines[at++] = -edge;
      lines[at++] = offset;
      lines[at++] = y;
      lines[at++] = edge;
      lines[at++] = -edge;
      lines[at++] = y;
      lines[at++] = offset;
      lines[at++] = edge;
      lines[at++] = y;
      lines[at++] = offset;
    }
    const gl = this.gl;
    model.gridCount = lines.length / 3;
    model.gridVao = required(gl.createVertexArray(), "the grid vertex array");
    model.gridBuffer = required(gl.createBuffer(), "the grid buffer");
    gl.bindVertexArray(model.gridVao);
    gl.bindBuffer(gl.ARRAY_BUFFER, model.gridBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, lines, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
  }

  private basis(): void {
    const cp = Math.cos(this.pitch);
    vec3.set(
      this.direction,
      cp * Math.sin(this.yaw),
      Math.sin(this.pitch),
      cp * Math.cos(this.yaw),
    );
    vec3.negate(this.forward, this.direction);
    vec3.cross(this.right, this.forward, UP);
    vec3.normalize(this.right, this.right);
    vec3.cross(this.up, this.right, this.forward);
  }

  private fittingDistance(): number {
    this.basis();
    const vertical = HALF_FOV_TAN / 1.18;
    const horizontal = vertical * this.aspect;
    let fit = 0.1;
    for (let corner = 0; corner < 8; corner++) {
      const x = (corner & 1) === 0 ? this.minimum[0] : this.maximum[0];
      const y = (corner & 2) === 0 ? this.minimum[1] : this.maximum[1];
      const z = (corner & 4) === 0 ? this.minimum[2] : this.maximum[2];
      const depth =
        x * this.forward[0] + y * this.forward[1] + z * this.forward[2];
      const across = x * this.right[0] + y * this.right[1] + z * this.right[2];
      const above = x * this.up[0] + y * this.up[1] + z * this.up[2];
      fit = Math.max(
        fit,
        Math.abs(above) / vertical - depth,
        Math.abs(across) / horizontal - depth,
      );
    }
    return fit;
  }

  private pan(dx: number, dy: number): void {
    this.basis();
    const scale = (2 * this.distance * HALF_FOV_TAN) / this.cssHeight;
    vec3.scaleAndAdd(this.target, this.target, this.right, -dx * scale);
    vec3.scaleAndAdd(this.target, this.target, this.up, dy * scale);
    this.invalidate();
  }

  private orbit(dx: number, dy: number): void {
    this.yaw = (this.yaw - dx * 0.008) % (Math.PI * 2);
    this.pitch = Math.max(-1.5, Math.min(1.5, this.pitch + dy * 0.008));
    this.invalidate();
  }

  private syncSize(): boolean {
    const width = this.canvas.clientWidth;
    const height = this.canvas.clientHeight;
    if (width <= 0 || height <= 0) return false;
    this.cssHeight = height;
    const dpr = Math.max(0.25, Math.min(window.devicePixelRatio || 1, 2));
    const scale = Math.min(
      dpr,
      this.maxWidth / width,
      this.maxHeight / height,
      Math.sqrt(MAX_RENDER_PIXELS / width / height),
    );
    const pixelsX = Math.max(1, Math.floor(width * scale));
    const pixelsY = Math.max(1, Math.floor(height * scale));
    const aspect = pixelsX / pixelsY;
    if (aspect !== this.aspect) {
      this.aspect = aspect;
      if (this.model) {
        const ratio = this.distance / this.fittedDistance;
        this.fittedDistance = this.fittingDistance();
        this.distance = this.fittedDistance * ratio;
      }
    }
    if (this.canvas.width !== pixelsX) this.canvas.width = pixelsX;
    if (this.canvas.height !== pixelsY) this.canvas.height = pixelsY;
    return true;
  }

  // An sRGB attachment blends in linear space while preserving dark-color
  // precision. Sampling it decodes to linear before the output shader encodes
  // for the canvas; encoding in the scene shader would blend in the wrong space.
  private createTargets(width: number, height: number): Targets {
    const gl = this.gl;
    const targets: Targets = {
      width,
      height,
      color: null,
      resolve: null,
      draw: null,
      depth: null,
      multisampleColor: null,
    };
    try {
      targets.color = required(gl.createTexture(), "the linear color target");
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, targets.color);
      gl.texStorage2D(gl.TEXTURE_2D, 1, gl.SRGB8_ALPHA8, width, height);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      targets.resolve = required(
        gl.createFramebuffer(),
        "the color framebuffer",
      );
      gl.bindFramebuffer(gl.FRAMEBUFFER, targets.resolve);
      gl.framebufferTexture2D(
        gl.FRAMEBUFFER,
        gl.COLOR_ATTACHMENT0,
        gl.TEXTURE_2D,
        targets.color,
        0,
      );
      if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE)
        throw new Error(
          "The graphics device cannot create the linear color framebuffer.",
        );
      targets.draw = targets.resolve;
      targets.depth = required(gl.createRenderbuffer(), "the depth target");
      gl.bindRenderbuffer(gl.RENDERBUFFER, targets.depth);
      if (this.samples > 1) {
        gl.renderbufferStorageMultisample(
          gl.RENDERBUFFER,
          this.samples,
          gl.DEPTH_COMPONENT24,
          width,
          height,
        );
        targets.draw = required(
          gl.createFramebuffer(),
          "the multisample framebuffer",
        );
        gl.bindFramebuffer(gl.FRAMEBUFFER, targets.draw);
        targets.multisampleColor = required(
          gl.createRenderbuffer(),
          "the multisample color target",
        );
        gl.bindRenderbuffer(gl.RENDERBUFFER, targets.multisampleColor);
        gl.renderbufferStorageMultisample(
          gl.RENDERBUFFER,
          this.samples,
          gl.SRGB8_ALPHA8,
          width,
          height,
        );
        gl.framebufferRenderbuffer(
          gl.FRAMEBUFFER,
          gl.COLOR_ATTACHMENT0,
          gl.RENDERBUFFER,
          targets.multisampleColor,
        );
      } else {
        gl.renderbufferStorage(
          gl.RENDERBUFFER,
          gl.DEPTH_COMPONENT24,
          width,
          height,
        );
      }
      gl.framebufferRenderbuffer(
        gl.FRAMEBUFFER,
        gl.DEPTH_ATTACHMENT,
        gl.RENDERBUFFER,
        targets.depth,
      );
      this.checkGraphics("allocate the drawing surface");
      if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE)
        throw new Error(
          "The graphics device cannot create the schematic drawing surface.",
        );
      return targets;
    } catch (error) {
      this.releaseTargets(targets);
      throw error;
    }
  }

  private readonly drawFrame = (): void => {
    this.frame = 0;
    if (this.disposed || this.contextLost || this.failed || !this.pipeline)
      return;
    const gl = this.gl;
    try {
      if (!this.syncSize()) return;
      const width = this.canvas.width;
      const height = this.canvas.height;
      if (
        !this.targets ||
        this.targets.width !== width ||
        this.targets.height !== height
      ) {
        this.releaseTargets(this.targets);
        this.targets = null;
        this.targets = this.createTargets(width, height);
      }
      const targets = this.targets;
      const pipeline = this.pipeline;
      gl.bindFramebuffer(gl.FRAMEBUFFER, targets.draw);
      gl.viewport(0, 0, width, height);
      gl.enable(gl.DEPTH_TEST);
      gl.depthMask(true);
      gl.disable(gl.BLEND);
      gl.disable(gl.CULL_FACE);
      gl.clearColor(0.00335, 0.00518, 0.00802, 1);
      gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
      if (this.model) {
        this.basis();
        vec3.scaleAndAdd(this.eye, this.target, this.direction, this.distance);
        mat4.lookAt(this.view, this.eye, this.target, UP);
        mat4.perspective(
          this.projection,
          FOV,
          this.aspect,
          Math.max(this.fittedDistance / 1000, 0.01),
          this.fittedDistance * 24,
        );
        mat4.multiply(this.mvp, this.projection, this.view);
        gl.useProgram(pipeline.scene);
        gl.uniformMatrix4fv(pipeline.matrix, false, this.mvp);
        gl.activeTexture(gl.TEXTURE0);
        // The grid shader does not sample, but a non-feedback sampler binding is
        // still needed when the resolved color texture was bound by the last frame.
        gl.bindTexture(gl.TEXTURE_2D, this.model.textures[0]);
        if (this.gridVisible) {
          gl.uniform1i(pipeline.grid, 1);
          gl.uniform3f(pipeline.offset, 0, 0, 0);
          gl.bindVertexArray(this.model.gridVao);
          gl.drawArrays(gl.LINES, 0, this.model.gridCount);
        }
        gl.uniform1i(pipeline.grid, 0);
        gl.uniform3f(
          pipeline.offset,
          -this.centre[0],
          -this.centre[1],
          -this.centre[2],
        );
        for (const part of this.model.parts)
          if (part.alphaMode !== 2) this.drawPart(part, pipeline);
        gl.enable(gl.BLEND);
        gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
        gl.depthMask(false);
        for (const part of this.model.parts)
          if (part.alphaMode === 2) this.drawPart(part, pipeline);
        gl.depthMask(true);
        gl.disable(gl.BLEND);
      }
      if (targets.draw !== targets.resolve) {
        gl.bindFramebuffer(gl.READ_FRAMEBUFFER, targets.draw);
        gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, targets.resolve);
        gl.blitFramebuffer(
          0,
          0,
          width,
          height,
          0,
          0,
          width,
          height,
          gl.COLOR_BUFFER_BIT,
          gl.NEAREST,
        );
      }
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.disable(gl.DEPTH_TEST);
      gl.disable(gl.CULL_FACE);
      gl.useProgram(pipeline.output);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, targets.color);
      gl.bindVertexArray(pipeline.fullscreen);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      gl.bindVertexArray(null);
      this.checkGraphics("draw this schematic");
    } catch (error) {
      this.failed = true;
      this.onError(errorOf(error).message);
    }
  };

  private drawPart(part: Part, pipeline: Pipeline): void {
    const gl = this.gl;
    if (part.alphaMode === 0) gl.enable(gl.CULL_FACE);
    else gl.disable(gl.CULL_FACE);
    gl.uniform1i(pipeline.alphaMode, part.alphaMode);
    gl.bindTexture(gl.TEXTURE_2D, part.texture);
    gl.bindVertexArray(part.vao);
    gl.drawElements(gl.TRIANGLES, part.count, gl.UNSIGNED_INT, 0);
  }

  private invalidate(): void {
    if (!this.frame && !this.disposed && !this.contextLost && !this.failed)
      this.frame = requestAnimationFrame(this.drawFrame);
  }

  private cancelFrame(): void {
    if (this.frame) cancelAnimationFrame(this.frame);
    this.frame = 0;
  }

  private checkGraphics(action: string): void {
    const gl = this.gl;
    const code = gl.getError();
    if (code === gl.NO_ERROR) return;
    // Drain the finite error flags so a failed upload cannot poison a later load.
    for (let i = 0; i < 8 && gl.getError() !== gl.NO_ERROR; i++) {
      /* drain */
    }
    if (code === gl.CONTEXT_LOST_WEBGL)
      throw new Error(
        "The graphics context was lost. Reopen the schematic after the graphics device recovers.",
      );
    throw new Error(
      `The graphics device could not ${action} (WebGL error 0x${code.toString(16)}). The model or window may exceed available graphics memory.`,
    );
  }

  private releaseModel(model: Model | null): void {
    if (!model || model.released) return;
    model.released = true;
    const gl = this.gl;
    for (const part of model.parts) {
      gl.deleteVertexArray(part.vao);
      for (const buffer of part.buffers) gl.deleteBuffer(buffer);
      part.buffers.length = 0;
    }
    for (const texture of model.textures) gl.deleteTexture(texture);
    gl.deleteVertexArray(model.gridVao);
    gl.deleteBuffer(model.gridBuffer);
    model.parts.length = 0;
    model.textures.length = 0;
    model.gridVao = null;
    model.gridBuffer = null;
  }

  private cancelStaged(): void {
    for (const model of this.staged) this.releaseModel(model);
    this.staged.clear();
    while (this.pendingYields.length) this.pendingYields.shift()!();
  }

  private releaseTargets(targets: Targets | null): void {
    if (!targets) return;
    const gl = this.gl;
    if (targets.draw !== targets.resolve) gl.deleteFramebuffer(targets.draw);
    gl.deleteFramebuffer(targets.resolve);
    gl.deleteTexture(targets.color);
    gl.deleteRenderbuffer(targets.depth);
    gl.deleteRenderbuffer(targets.multisampleColor);
  }

  private releasePipeline(): void {
    if (!this.pipeline) return;
    this.gl.useProgram(null);
    this.gl.deleteProgram(this.pipeline.scene);
    this.gl.deleteProgram(this.pipeline.output);
    this.gl.deleteVertexArray(this.pipeline.fullscreen);
    this.pipeline = null;
  }

  private watchDpr(): void {
    this.dprQuery?.removeEventListener("change", this.onDprChange);
    this.dprQuery = window.matchMedia(
      `(resolution: ${window.devicePixelRatio || 1}dppx)`,
    );
    this.dprQuery.addEventListener("change", this.onDprChange);
  }

  private readonly onResize = (): void => this.invalidate();
  private readonly onDprChange = (): void => {
    this.watchDpr();
    this.invalidate();
  };
  private readonly onContextMenu = (event: Event): void =>
    event.preventDefault();

  private readonly onPointerDown = (event: PointerEvent): void => {
    if (this.pointerId !== null || event.button < 0 || event.button > 2) return;
    this.canvas.focus({ preventScroll: true });
    this.pointerId = event.pointerId;
    this.pointerButton = event.button;
    this.lastX = event.clientX;
    this.lastY = event.clientY;
    this.canvas.setPointerCapture(event.pointerId);
    event.preventDefault();
  };

  private readonly onPointerMove = (event: PointerEvent): void => {
    if (event.pointerId !== this.pointerId) return;
    const dx = event.clientX - this.lastX;
    const dy = event.clientY - this.lastY;
    this.lastX = event.clientX;
    this.lastY = event.clientY;
    if (!this.model || (dx === 0 && dy === 0)) return;
    if (this.pointerButton === 0) this.orbit(dx, dy);
    else this.pan(dx, dy);
  };

  private endPointer(): void {
    const pointer = this.pointerId;
    this.pointerId = null;
    if (pointer !== null && this.canvas.hasPointerCapture(pointer))
      this.canvas.releasePointerCapture(pointer);
  }

  private readonly onPointerEnd = (event: PointerEvent): void => {
    if (event.pointerId === this.pointerId) this.endPointer();
  };

  private readonly onWheel = (event: WheelEvent): void => {
    if (!this.model) return;
    event.preventDefault();
    const unit =
      event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? this.cssHeight : 1;
    this.zoom(
      Math.exp(Math.max(-1200, Math.min(1200, event.deltaY * unit)) * 0.001),
    );
  };

  private readonly onKeyDown = (event: KeyboardEvent): void => {
    if (!this.model || event.ctrlKey || event.altKey || event.metaKey) return;
    let dx = 0;
    let dy = 0;
    switch (event.key) {
      case "ArrowLeft":
        dx = -18;
        break;
      case "ArrowRight":
        dx = 18;
        break;
      case "ArrowUp":
        dy = -18;
        break;
      case "ArrowDown":
        dy = 18;
        break;
      case "+":
      case "=":
        this.zoom(0.88);
        break;
      case "-":
      case "_":
        this.zoom(1.12);
        break;
      case "f":
      case "F":
      case "Home":
        this.fit();
        break;
      default:
        return;
    }
    event.preventDefault();
    if (dx !== 0 || dy !== 0) {
      if (event.shiftKey) this.pan(dx, dy);
      else this.orbit(dx, dy);
    }
  };

  private readonly onContextLost = (event: Event): void => {
    event.preventDefault();
    this.contextLost = true;
    this.generation++;
    this.cancelFrame();
    this.cancelStaged();
    this.releaseModel(this.model);
    this.model = null;
    this.releaseTargets(this.targets);
    this.targets = null;
    this.releasePipeline();
    this.onError(
      "The graphics context was lost. Reopen the schematic after the graphics device recovers.",
    );
  };

  private readonly onContextRestored = (): void => {
    if (this.disposed) return;
    this.contextLost = false;
    this.failed = false;
    try {
      this.initialize();
      this.invalidate();
      this.onError(
        "The graphics device recovered. Reopen the schematic to restore its preview.",
      );
    } catch (error) {
      this.failed = true;
      this.onError(errorOf(error).message);
    }
  };
}
