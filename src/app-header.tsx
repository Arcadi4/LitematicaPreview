import {
  Badge,
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
  onNativeAction: (command: NativeCommand) => void
  onOpenDialog: (dialog: DialogKind) => void
}

export function AppHeader({ actionBusy, onNativeAction, onOpenDialog }: AppHeaderProps) {
  return (
    <header className="flex-none flex items-center justify-between gap-4 min-h-14 sm:min-h-16 px-4 py-2.5 sm:px-6 sm:py-3 bg-surface border-b border-border">
      <div className="flex items-center gap-2.5 sm:gap-3 text-sm sm:text-base font-semibold tracking-tight">
        <img className="object-contain flex-none" src={icon} width="30" height="30" alt="" />
        <span>{appName}</span>
        <Badge
          appearance="outline"
          className="ml-1.5! font-normal! text-muted! hidden! sm:inline-flex!"
        >
          Desktop
        </Badge>
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
          </MenuList>
        </MenuPopover>
      </Menu>
    </header>
  )
}
