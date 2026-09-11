# Architecture Convergence

This document describes the architecture improvements implemented to establish consistent patterns across the ModelSwitch admin UI.

## Overview

The goal of this refactoring is to reduce code duplication, establish shared conventions for loading/error/empty states, and provide a reusable panel layout structure without changing product behavior or visual design.

## New Shared Components

### 1. PanelLayout (`src/components/ui/PanelLayout.tsx`)

A reusable shell component that provides a consistent structure for panels:

```tsx
<PanelLayout
  title="Panel Title"
  icon={IconComponent}
  actions={<>Action buttons</>}
  toolbar={<>Filter controls</>}
  onRefresh={handleRefresh}
  refreshing={isRefreshing}
>
  {/* Panel content */}
</PanelLayout>
```

**Features:**
- Title area with optional icon
- Actions slot for buttons (create, export, etc.)
- Optional toolbar slot for filters/search
- Optional refresh button
- Consistent spacing and layout

### 2. LoadingState (`src/components/ui/LoadingState.tsx`)

Standardized loading indicator:

```tsx
<LoadingState message="Loading data..." icon={KeyRound} />
```

**Features:**
- Animated spinner
- Optional message
- Optional icon
- Consistent styling

### 3. ErrorState (`src/components/ui/ErrorState.tsx`)

Standardized error display:

```tsx
<ErrorState
  title="Error Title"
  message="Error details"
  onRetry={handleRetry}
  retryLabel="Try Again"
/>
```

**Features:**
- Error icon (customizable)
- Title and message
- Optional retry button
- Consistent styling

### 4. EmptyState (existing, `src/components/ui/EmptyState.tsx`)

Already existed, now consistently used across panels for "no data" states.

## Migrated Panels

The following high-traffic panels have been migrated to use the new architecture:

### 1. Virtual Keys Panel ✅
- **Before:** Custom loading div, SectionHeader, inline toolbar
- **After:** PanelLayout with LoadingState, actions and toolbar slots
- **Benefits:** Cleaner structure, consistent loading state

### 2. MCP Servers Panel ✅
- **Before:** Custom loading div, panel-header, inline empty state
- **After:** PanelLayout with LoadingState, EmptyState component
- **Benefits:** Consistent structure, reusable components

### 3. Log Viewer ✅
- **Before:** panel-loading div, panel-header, inline empty states
- **After:** PanelLayout with LoadingState, EmptyState components
- **Benefits:** Unified structure, cleaner code

### 4. Settings Panel ✅
- **Before:** SectionHeader, panel-loading div
- **After:** PanelLayout with LoadingState
- **Benefits:** Consistent with other panels, cleaner header

### 5. Channels Panel ✅
- **Before:** Custom panel-header, panel-loading div, complex toolbar
- **After:** PanelLayout with LoadingState, toolbar slot
- **Benefits:** Separated concerns, reusable structure

## Navigation Module Extraction

Extracted tab configuration from `App.tsx` to `src/app/navigation.ts`:

- **Type Definitions:** `TabId`, `TabConfig`, `TabGroup`
- **Configuration:** `getTabGroups()` function
- **Constants:** `ALL_TAB_IDS` for keyboard shortcuts

**Benefits:**
- Easier to maintain navigation structure
- Single source of truth for tab configuration
- Cleaner App.tsx (reduced ~40 lines)

## Style Improvements

### Design Tokens Usage
- Leveraged existing `tokens.css` variables
- Reduced inline `style={{...}}` usage where trivial
- Created `src/styles/components/panel.css` for shared panel styles

### CSS Organization
- Added new component styles to `ui.css`
- Created dedicated `panel.css` for shared panel patterns
- Preserved existing styles to avoid breaking changes

## Testing

Added comprehensive unit tests for new components:
- `LoadingState.test.tsx` - Loading state rendering
- `ErrorState.test.tsx` - Error display and retry behavior
- `PanelLayout.test.tsx` - Layout structure and interactions

## Migration Strategy

### Conservative Approach
- No visual design changes
- No behavior changes
- Preserved all existing functionality
- Backward compatible (legacy patterns still work)

### Gradual Migration
Other panels can be migrated incrementally using the same patterns:

```tsx
// Before
<section>
  <div className="panel-header">
    <h2 className="panel-title">Title</h2>
    <button onClick={refresh}>Refresh</button>
  </div>
  {loading && <div className="panel-loading">Loading...</div>}
  {/* content */}
</section>

// After
<PanelLayout
  title="Title"
  icon={Icon}
  onRefresh={refresh}
>
  {loading ? (
    <LoadingState message="Loading..." />
  ) : (
    /* content */
  )}
</PanelLayout>
```

## I18n Support

Added translations for new UI elements:
- `common.retryAction` - Retry button label (en: "Retry", zh: "重试")

All existing translations preserved and reused.

## Future Improvements

Potential next steps (not included in this PR):
1. Migrate remaining panels (Dashboard, Cost, Quota, etc.)
2. Extract common filter/search patterns into shared components
3. Standardize modal/dialog patterns
4. Create a component library documentation

## Verification

### Manual Testing
- Navigate to each migrated panel
- Verify loading states display correctly
- Test refresh functionality
- Verify empty states when no data
- Check responsiveness on mobile

### Visual Regression
No visual changes expected - panels should look identical to before.

### Functional Testing
All existing features should work unchanged:
- Create/edit/delete operations
- Search and filtering
- Pagination
- Batch operations
- Modal interactions

## Files Changed

### New Files
- `src/components/ui/LoadingState.tsx`
- `src/components/ui/LoadingState.test.tsx`
- `src/components/ui/ErrorState.tsx`
- `src/components/ui/ErrorState.test.tsx`
- `src/components/ui/PanelLayout.tsx`
- `src/components/ui/PanelLayout.test.tsx`
- `src/app/navigation.ts`
- `src/styles/components/panel.css`
- `docs/ARCHITECTURE_CONVERGENCE.md`

### Modified Files
- `src/components/virtualkeys/VirtualKeysPanel.tsx`
- `src/components/mcp/McpServersPanel.tsx`
- `src/components/LogViewer.tsx`
- `src/components/SettingsPanel.tsx`
- `src/components/channel/ChannelPanel.tsx`
- `src/App.tsx`
- `src/styles/ui.css`
- `src/i18n/locales/en.json`
- `src/i18n/locales/zh.json`

## Metrics

- **New Components:** 3 (LoadingState, ErrorState, PanelLayout)
- **Panels Migrated:** 5 (VirtualKeys, MCP, Logs, Settings, Channels)
- **Tests Added:** 3 test files, ~20 test cases
- **Lines Reduced:** ~100+ from App.tsx and panel files
- **Code Duplication:** Eliminated loading/error/empty state duplication
