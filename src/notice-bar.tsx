import {
  Button,
  MessageBar,
  MessageBarActions,
  MessageBarBody,
  MessageBarGroup,
  MessageBarTitle,
} from "@fluentui/react-components"
import { Dismiss20Regular } from "@fluentui/react-icons"
import type { Notice } from "./app-model"

export interface NoticeBarProps {
  /** `null` plays the exit transition; a notice with different content replaces the shown one. */
  notice: Notice | null
  onDismiss: () => void
  className?: string
}

export function NoticeBar({ notice, onDismiss, className = "" }: NoticeBarProps) {
  return (
    // A single grid cell overlaps the outgoing and incoming bars instead of stacking them.
    <MessageBarGroup
      animate="both"
      className={`grid overflow-hidden *:col-start-1 *:row-start-1 ${className}`}
    >
      {(notice ? [notice] : []).map((shown) => (
        <MessageBar
          key={`${shown.intent}:${shown.message}`}
          intent={shown.intent}
          className="rounded-none! break-words"
        >
          <MessageBarBody>
            <MessageBarTitle>
              {shown.intent === "error"
                ? "Something went wrong"
                : shown.intent === "success"
                  ? "Done"
                  : "Please note"}
            </MessageBarTitle>
            {shown.message}
          </MessageBarBody>
          <MessageBarActions
            containerAction={
              <Button
                appearance="transparent"
                icon={<Dismiss20Regular />}
                aria-label="Dismiss message"
                onClick={onDismiss}
              />
            }
          />
        </MessageBar>
      ))}
    </MessageBarGroup>
  )
}
