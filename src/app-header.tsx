import {
  Badge,
  Button,
  Menu,
  MenuDivider,
  MenuGroup,
  MenuGroupHeader,
  MenuItem,
  MenuItemRadio,
  MenuList,
  MenuPopover,
  MenuTrigger,
} from "@fluentui/react-components"
import {
  Info20Regular,
  Keyboard20Regular,
  MoreHorizontal20Regular,
  Settings20Regular,
} from "@fluentui/react-icons"
import icon from "../Assets/app-ui.png"
import {
  appName,
  isTheme,
  type DialogKind,
  type NativeCommand,
  type ThemePreference,
} from "./app-model"

export interface AppHeaderProps {
  actionBusy: boolean
  onNativeAction: (command: NativeCommand) => void
  onOpenDialog: (dialog: DialogKind) => void
  onThemeChange: (theme: ThemePreference) => void
  theme: ThemePreference
}

export function AppHeader({
  actionBusy,
  onNativeAction,
  onOpenDialog,
  onThemeChange,
  theme,
}: AppHeaderProps) {
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
      <Menu
        checkedValues={{ theme: [theme] }}
        onCheckedValueChange={(_, data) => {
          const value = data.checkedItems[0]
          if (data.name === "theme" && isTheme(value)) onThemeChange(value)
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
            <MenuItem icon={<Settings20Regular />} onClick={() => onOpenDialog("settings")}>
              Preview settings
            </MenuItem>
            <MenuItem icon={<Keyboard20Regular />} onClick={() => onOpenDialog("controls")}>
              Controls and shortcuts
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
  )
}
