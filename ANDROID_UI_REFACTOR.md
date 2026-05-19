# Android UI Refactor - iOS Parity

## Summary
Complete refactor of Android UI to match iOS design with:
- Bottom navigation tab bar with SVG icons (Status, Servers, Settings)
- Clean, minimal server list layout matching iOS
- Refactored modal dialogs with organized sections
- Fragment-based architecture for better code organization

## New Files Created

### Layouts
- `activity_main_new.xml` - Main activity with bottom navigation bar
- `fragment_status.xml` - Status/home tab showing VPN status and dashboard
- `fragment_servers.xml` - Servers tab with server list
- `fragment_settings.xml` - Settings tab with configuration options
- `dialog_new_server.xml` - Clean modal for creating/editing servers
- `item_server.xml` - Server list item with minimal design

### Kotlin Fragments
- `StatusFragment.kt` - Status tab (migrated from MainActivity)
- `ServersFragment.kt` - Servers list with add/edit/delete
- `SettingsFragment.kt` - Settings and about section
- `MainActivityNew.kt` - New main activity with bottom navigation

### Dialog Helpers
- `ServerDialogs.kt` - NewServerDialog, EditServerDialog, ServerMenuDialog

### Icons (SVG)
- `ic_home.svg` - Home/Status icon
- `ic_servers.svg` - Servers icon
- `ic_settings_nav.svg` - Settings icon
- `ic_checkmark.svg` - Selected indicator
- `ic_more.svg` - Menu button
- `ic_close.svg` - Close/back button

## Design Features

### Bottom Navigation Bar
- Clean, minimal tab bar at bottom (similar to iOS)
- 60dp height with 3 tabs: Status, Servers, Settings
- Active tab highlighted in dark gray (#111827)
- Inactive tabs in light gray (#9CA3AF)
- SVG icons for crisp rendering at all sizes

### Status Tab (Fragment)
- Same layout as original MainActivity
- Shows VPN status, connection metrics, and logs
- Clean card-based design with proper spacing

### Servers Tab
- Minimal list of saved server configurations
- Each server shows name and transport type
- Checkmark icon indicates selected server
- Add button (+) in header for new servers
- Context menu (⋮) for edit/delete
- Clean empty state when no servers exist
- Tap server to select it

### Server Dialog
- Organized sections: Profile Details, Server Info, Network, Advanced
- Clean input fields with proper labels
- TLS Fragmentation toggle with size field
- All fields have appropriate hints
- Save/Close buttons in header

### Settings Tab
- Default Protocol, Transport, TLS Fragmentation summary
- About section with links to Privacy, Terms, Support
- Version information
- Clean list-based layout

## Integration Steps

1. **Update AndroidManifest.xml** - Update MainActivity reference or add MainActivityNew

2. **Replace MainActivity.kt** - Either:
   - Rename MainActivityNew.kt to MainActivity.kt
   - Or update AndroidManifest.xml to point to MainActivityNew

3. **Update TunnelPreferences** - Ensure these methods exist:
   - `deleteConfiguration(context, configId)` - Delete a saved config
   - `saveSelectedConfigurationId(context, configId)` - Save selected server

4. **Add Missing Methods** to TunnelPreferences if needed:
   ```kotlin
   fun deleteConfiguration(context: Context, configId: String) {
       val prefs = context.getSharedPreferences(PREFS_SAVED_CONFIGURATIONS, Context.MODE_PRIVATE)
       prefs.edit().remove(configId).apply()
   }

   fun saveSelectedConfigurationId(context: Context, configId: String) {
       val prefs = context.getSharedPreferences(PREFS_MAIN, Context.MODE_PRIVATE)
       prefs.edit().putString("selected_config_id", configId).apply()
   }
   ```

5. **Update SettingsActivity** - Keep as-is or migrate to SettingsFragment if needed

6. **Test** - Verify:
   - Bottom navigation switching works
   - Server list displays properly
   - Adding/editing/deleting servers works
   - Status metrics update in real-time
   - Connection flow works through all tabs

## Design Principles Applied

✓ Minimal, clean design matching iOS
✓ Proper spacing and typography hierarchy
✓ SVG icons for crisp rendering
✓ Fragment-based for better organization
✓ No unnecessary UI elements
✓ Color consistency with existing design
✓ Touch-friendly tab bar (60dp height)
✓ Modal dialogs with organized sections

## Next Steps (Optional)

1. Create preferences for light/dark mode
2. Add animations for tab switching
3. Add pull-to-refresh on server list
4. Add search functionality to server list
5. Add server grouping by profile type
6. Add more metrics to status view
7. Add VPN protocol selection in settings
