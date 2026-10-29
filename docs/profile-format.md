# Optimaxer tweak profiles

Profiles store a named list of tweak IDs. They do not contain registry values, service states, undo records, hardware readings or a full system backup.

## Portable profile

The Tweaks page imports and exports one profile as UTF-8 JSON:

```json
{
  "format_version": 1,
  "name": "Daily",
  "tweak_ids": [
    "perf-animations"
  ]
}
```

`format_version` must be `1`. Names contain 1 to 64 characters and cannot contain control characters. A profile contains 1 to 512 unique tweak IDs. Files larger than 64 KiB are rejected.

## Import behavior

Import replaces the current tweak selection. It never applies a tweak. Unknown IDs, retired tweaks, opt-in tweaks and tweaks that do not fit the current hardware are skipped and counted. If two compatible tweaks conflict by writing the same setting, the profile is rejected. The user can edit the resulting selection, then must use the normal exact preview and confirmation to apply it.

This check runs against the current embedded catalog and current hardware traits on every import or profile selection. A profile does not make a tweak valid on a different PC.

## Local profiles

Optimaxer stores up to 20 named profiles in `%LOCALAPPDATA%\OrionStore\profiles.json`, using a versioned wrapper around the same profile format. Corrupt or unsupported data is reported and preserved; saving is blocked until the file is recovered. Local profile data is separate from tweak undo records.
