import {
  Button,
  Menu,
  MenuDivider,
  MenuGroup,
  MenuGroupHeader,
  MenuItem,
  MenuList,
  MenuPopover,
  MenuTrigger,
} from "@fluentui/react-components"
import { Info20Regular, MoreHorizontal20Regular, Settings20Regular } from "@fluentui/react-icons"

import icon from "../Assets/app-ui.png"
import { appName, type DialogKind, type NativeCommand } from "./app-model"

export interface AppHeaderProps {
  actionBusy: boolean
  fileAssociationsSupported: boolean
  onNativeAction: (command: NativeCommand) => void
  onOpenDialog: (dialog: DialogKind) => void
}

export function AppHeader({
  actionBusy,
  fileAssociationsSupported,
  onNativeAction,
  onOpenDialog,
}: AppHeaderProps) {
  return (
    <header className="bg-surface border-border flex min-h-14 flex-none items-center justify-between gap-4 border-b px-4 py-2.5 sm:min-h-16 sm:px-6 sm:py-3">
      <div className="flex items-center gap-2.5 text-sm font-semibold tracking-tight sm:gap-3 sm:text-base">
        <img className="flex-none object-contain" src={icon} width="30" height="30" alt="" />
        <span>{appName}</span>
      </div>
      <Menu>
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
            <MenuItem icon={<Settings20Regular />} onClick={() => onOpenDialog("settings")}>
              Settings
            </MenuItem>
            <MenuItem icon={<Info20Regular />} onClick={() => onOpenDialog("about")}>
              About and licenses
            </MenuItem>
            {fileAssociationsSupported && (
              <>
                <MenuDivider />
                <MenuGroup>
                  <MenuGroupHeader>File associations</MenuGroupHeader>
                  <MenuItem
                    disabled={actionBusy}
                    onClick={() => onNativeAction("register_associations")}
                  >
                    Set as default app…
                  </MenuItem>
                  <MenuItem
                    disabled={actionBusy}
                    onClick={() => onNativeAction("unregister_associations")}
                  >
                    Remove file associations
                  </MenuItem>
                </MenuGroup>
              </>
            )}
          </MenuList>
        </MenuPopover>
      </Menu>
    </header>
  )
}
