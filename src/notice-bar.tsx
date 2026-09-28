import {
  Button,
  MessageBar,
  MessageBarActions,
  MessageBarBody,
  MessageBarTitle,
} from "@fluentui/react-components"
import { Dismiss20Regular } from "@fluentui/react-icons"
import type { Notice } from "./app-model"

export interface NoticeBarProps {
  notice: Notice
  onDismiss: () => void
}

export function NoticeBar({ notice, onDismiss }: NoticeBarProps) {
  return (
    <MessageBar intent={notice.intent} className="flex-none rounded-none! break-words">
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
            onClick={onDismiss}
          />
        }
      />
    </MessageBar>
  )
}
