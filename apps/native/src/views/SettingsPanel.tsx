/**
 * `?view=settings` — keys/models/hotkeys panel. A later task fills this
 * in; for now it renders the translucent panel chrome so the window
 * isn't blank.
 */
export default function SettingsPanel() {
  return (
    <div className="h-full p-1">
      <div className="flex h-full items-center justify-center rounded-2xl border border-border bg-card/80 text-xs text-muted-foreground shadow-lg backdrop-blur">
        Settings
      </div>
    </div>
  );
}
