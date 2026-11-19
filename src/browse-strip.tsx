import { Button } from "@fluentui/react-components"
import {
  ChevronLeft20Regular,
  ChevronRight20Regular,
  Document20Regular,
} from "@fluentui/react-icons"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { useEffect, useRef, useState, type KeyboardEvent } from "react"

import { fileName } from "./app-model"

export type BrowseSelection = { paths: string[]; index: number }

export function parseBrowseSelection(value: unknown): BrowseSelection | null {
  if (typeof value !== "object" || value === null) return null
  const { paths, index } = value as { paths?: unknown; index?: unknown }
  if (!Array.isArray(paths) || paths.length === 0) return null
  if (!paths.every((path) => typeof path === "string" && path.length > 0)) return null
  if (typeof index !== "number" || !Number.isInteger(index) || index < 0 || index >= paths.length)
    return null
  return { paths: paths as string[], index }
}

const ITEM_WIDTH = 104
const ITEM_GAP = 4
const SLOT = ITEM_WIDTH + ITEM_GAP
const OVERSCAN = 2
const MAX_THUMBNAIL_LOOKUPS = 3
const MAX_CACHED_THUMBNAILS = 120

function withThumbnail(
  previous: ReadonlyMap<string, string | null>,
  path: string,
  url: string | null,
) {
  const next = new Map(previous)
  next.delete(path)
  next.set(path, url)
  while (next.size > MAX_CACHED_THUMBNAILS) {
    const oldest = next.keys().next().value
    if (oldest === undefined) break
    next.delete(oldest)
  }
  return next
}

function withoutThumbnail(previous: ReadonlyMap<string, string | null>, path: string) {
  if (!previous.has(path)) return previous
  const next = new Map(previous)
  next.delete(path)
  return next
}

export interface BrowseStripProps {
  onNext: () => void
  onPrevious: () => void
  onSelect: (index: number) => void
  selection: BrowseSelection
}

export function BrowseStrip({ onNext, onPrevious, onSelect, selection }: BrowseStripProps) {
  const { index, paths } = selection
  const listRef = useRef<HTMLDivElement>(null)
  const pending = useRef(new Set<string>())
  const requestTokens = useRef(new Map<string, number>())
  const inFlight = useRef(0)
  const focusIndex = useRef<number | null>(null)
  const mounted = useRef(true)
  const [thumbnails, setThumbnails] = useState<ReadonlyMap<string, string | null>>(() => new Map())
  const [revision, setRevision] = useState(0)
  const [viewport, setViewport] = useState({ left: 0, width: 0 })

  useEffect(() => {
    mounted.current = true
    let active = true
    let dispose: (() => void) | undefined
    void listen<{ path?: unknown }>("browse-thumbnail", ({ payload }) => {
      if (!active || !payload || typeof payload.path !== "string") return
      const path = payload.path
      // A completion invalidates any lookup that started before it.
      if (pending.current.has(path))
        requestTokens.current.set(path, (requestTokens.current.get(path) ?? 0) + 1)
      setThumbnails((previous) => withoutThumbnail(previous, path))
    })
      .then((unlisten) => {
        if (active) dispose = unlisten
        else unlisten()
      })
      .catch(() => {})
    return () => {
      active = false
      mounted.current = false
      dispose?.()
    }
  }, [])

  useEffect(() => {
    const element = listRef.current
    if (!element) return
    let frame: number | null = null
    const report = () => {
      frame = null
      const left = element.scrollLeft
      const width = element.clientWidth
      setViewport((previous) =>
        previous.left === left && previous.width === width ? previous : { left, width },
      )
    }
    const schedule = () => {
      if (frame === null) frame = requestAnimationFrame(report)
    }
    report()
    element.addEventListener("scroll", schedule, { passive: true })
    const observer = new ResizeObserver(schedule)
    observer.observe(element)
    return () => {
      element.removeEventListener("scroll", schedule)
      observer.disconnect()
      if (frame !== null) cancelAnimationFrame(frame)
    }
  }, [])

  let startIndex = Math.max(0, Math.floor(viewport.left / SLOT) - OVERSCAN)
  let endIndex = Math.min(
    paths.length,
    Math.ceil((viewport.left + Math.max(viewport.width, SLOT)) / SLOT) + OVERSCAN,
  )
  const span = Math.max(1, endIndex - startIndex)
  if (index < startIndex) {
    startIndex = Math.max(0, index - OVERSCAN)
    endIndex = Math.min(paths.length, startIndex + span)
  } else if (index >= endIndex) {
    endIndex = Math.min(paths.length, index + OVERSCAN + 1)
    startIndex = Math.max(0, endIndex - span)
  }

  useEffect(() => {
    if (!mounted.current) return
    const visible: number[] = []
    for (let item = startIndex; item < endIndex; item += 1) visible.push(item)
    visible.sort((a, b) => Math.abs(a - index) - Math.abs(b - index) || a - b)
    for (const item of visible) {
      if (inFlight.current >= MAX_THUMBNAIL_LOOKUPS) break
      const path = paths[item]
      if (thumbnails.has(path) || pending.current.has(path)) continue
      const token = (requestTokens.current.get(path) ?? 0) + 1
      requestTokens.current.set(path, token)
      pending.current.add(path)
      inFlight.current += 1
      void invoke<string | null>("browse_thumbnail", { path })
        .then((url) => {
          inFlight.current -= 1
          pending.current.delete(path)
          if (!mounted.current) return
          if (requestTokens.current.get(path) !== token) {
            requestTokens.current.delete(path)
            setRevision((value) => value + 1)
            return
          }
          requestTokens.current.delete(path)
          setThumbnails((previous) =>
            withThumbnail(previous, path, typeof url === "string" ? url : null),
          )
        })
        .catch(() => {
          inFlight.current -= 1
          pending.current.delete(path)
          if (!mounted.current) return
          if (requestTokens.current.get(path) !== token) {
            requestTokens.current.delete(path)
            setRevision((value) => value + 1)
            return
          }
          requestTokens.current.delete(path)
          setThumbnails((previous) => withThumbnail(previous, path, null))
        })
    }
  }, [endIndex, index, paths, revision, startIndex, thumbnails])

  useEffect(() => {
    const element = listRef.current
    if (!element) return
    const left = index * SLOT
    const right = left + ITEM_WIDTH
    if (left < element.scrollLeft) element.scrollLeft = left
    else if (right > element.scrollLeft + element.clientWidth)
      element.scrollLeft = right - element.clientWidth
  }, [index, paths.length])

  useEffect(() => {
    const target = focusIndex.current
    if (target === null) return
    const element = listRef.current?.querySelector<HTMLElement>(`[data-browse-index="${target}"]`)
    if (!element) return
    focusIndex.current = null
    element.focus({ preventScroll: true })
  }, [endIndex, index, startIndex])

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return
    let target: number
    switch (event.key) {
      case "ArrowLeft":
        target = index - 1
        break
      case "ArrowRight":
        target = index + 1
        break
      case "Home":
        target = 0
        break
      case "End":
        target = paths.length - 1
        break
      default:
        return
    }
    event.preventDefault()
    if (target < 0 || target >= paths.length || target === index) return
    focusIndex.current = target
    onSelect(target)
  }

  const totalWidth = paths.length * ITEM_WIDTH + (paths.length - 1) * ITEM_GAP

  return (
    <nav
      className="border-border bg-surface flex flex-none items-center gap-1.5 border-t px-2 py-2 sm:gap-2 sm:px-3"
      aria-label="Schematic browser"
    >
      <div className="flex shrink-0 items-center gap-0.5">
        <Button
          appearance="subtle"
          size="small"
          className="shrink-0"
          disabled={index === 0}
          icon={<ChevronLeft20Regular />}
          onClick={onPrevious}
          aria-label="Previous schematic"
          aria-keyshortcuts="ArrowLeft"
          title="Previous schematic (←)"
        />
        <span
          className="text-muted min-w-9 text-center text-xs tabular-nums"
          aria-live="polite"
          aria-atomic="true"
        >
          {index + 1} / {paths.length}
        </span>
        <Button
          appearance="subtle"
          size="small"
          className="shrink-0"
          disabled={index === paths.length - 1}
          icon={<ChevronRight20Regular />}
          onClick={onNext}
          aria-label="Next schematic"
          aria-keyshortcuts="ArrowRight"
          title="Next schematic (→)"
        />
      </div>
      <div
        ref={listRef}
        className="min-w-0 flex-1 overflow-x-auto overflow-y-hidden [overscroll-behavior-x:contain]"
        role="listbox"
        aria-label="Schematics in this selection"
        aria-orientation="horizontal"
        tabIndex={-1}
        onKeyDown={onKeyDown}
      >
        <div className="relative h-[68px]" style={{ width: totalWidth }}>
          <div
            role="none"
            className="absolute inset-y-0 flex gap-1"
            style={{ transform: `translateX(${startIndex * SLOT}px)` }}
          >
            {paths.slice(startIndex, endIndex).map((path, offset) => {
              const item = startIndex + offset
              const active = item === index
              const thumbnail = thumbnails.get(path)
              return (
                <button
                  key={path}
                  type="button"
                  role="option"
                  aria-selected={active}
                  aria-posinset={item + 1}
                  aria-setsize={paths.length}
                  aria-label={`${fileName(path)}, ${item + 1} of ${paths.length}`}
                  tabIndex={active ? 0 : -1}
                  data-browse-index={item}
                  title={path}
                  onClick={() => onSelect(item)}
                  className={`focus-visible:outline-accent flex h-full shrink-0 flex-col items-center justify-start gap-1 rounded-md border px-1 py-1 text-[11px] leading-tight focus-visible:outline-2 focus-visible:-outline-offset-2 ${
                    active
                      ? "border-accent bg-accent-soft text-text"
                      : "bg-surface-secondary text-muted hover:bg-surface-muted hover:text-text border-transparent"
                  }`}
                  style={{ width: ITEM_WIDTH }}
                >
                  <span className="bg-surface-muted flex h-10 w-16 shrink-0 items-center justify-center overflow-hidden rounded-sm">
                    {thumbnail ? (
                      <img
                        alt=""
                        className="h-full w-full object-contain"
                        decoding="async"
                        draggable={false}
                        loading="lazy"
                        src={thumbnail}
                      />
                    ) : (
                      <Document20Regular />
                    )}
                  </span>
                  <span className="w-full overflow-hidden text-ellipsis whitespace-nowrap">
                    {fileName(path)}
                  </span>
                </button>
              )
            })}
          </div>
        </div>
      </div>
    </nav>
  )
}
