# Rainmill Mobile

App management for a [Laterite](https://laterite.rs) application.

- **Apps and platforms**: an app with its Android, iOS and web platforms, each build variant registered by package name, bundle ID or hostname, with an environment.
- **Policies**: feature flags, maintenance, required and suggested updates, and banners, decided by one engine and targeted by platform, environment, build range, locale, share of installs and time.
- **Releases**: version, build, track, state and changelog on record.
- **Devices**: each install that checks in, with its version, build, system and locale.
- **Links that open the app**: the Apple and Android association files, served from the registered identifiers.

## Requirements

Laterite 0.11 and `rainmill-vault`. A credential in the vault can belong to one app (scope `rainmill.mobile.app`).

## Installation

```toml
[dependencies]
rainmill-mobile = "0.1"
```

Add `rainmill_mobile::module()` to the application's modules, after `rainmill_vault::module()`.

## The check-in

```http
POST /mobile/v1/hello
Content-Type: application/json

{ "app": "acme", "platform": "android", "identifier": "com.acme.app",
  "install_id": "5f0c…", "build": 42, "version": "1.4.0",
  "os_version": "15", "device_model": "Pixel 9", "locale": "en-IN", "timezone": "Asia/Kolkata" }
```

```json
{
  "update": { "level": "soft", "message": "", "store_url": "https://acme.com/get", "latest_build": 45, "latest_version": "1.5.0" },
  "maintenance": { "message": "Back at 6 pm.", "until": "2026-11-20T12:30:00Z" },
  "flags": { "new_feed": true, "max_uploads": 5 },
  "banners": [{ "key": "diwali", "message": "Happy Diwali!" }],
  "environment": "production",
  "checked_at": "2026-11-18T10:00:00Z"
}
```

| Field | Values |
| --- | --- |
| `update.level` | `none`, `soft` (may dismiss) or `hard` (must update) |
| `maintenance` | `null`, or the message to show while the app is closed |
| `flags` | Each flag by name: `true`, `false`, a number, text or JSON |
| `banners` | Messages to show once each, by `key` |

| Status | Meaning |
| --- | --- |
| `200` | The verdict |
| `400` | `install_id` or `build` is missing |
| `403` | The identifier is not a registered platform of the app |
| `503` | Try again; the app keeps its last verdict |

## Policies

| Kind | Value | Message |
| --- | --- | --- |
| Feature flag | `true`, `false`, a number, text or JSON; `true` when empty | |
| Maintenance | | What the person reads |
| Update | `soft` or `hard` | What the person reads |
| Banner | | What the person reads |

| Condition | Applies to |
| --- | --- |
| Platform | `android`, `ios` or `web` |
| Environment | `production`, `staging` or `development` |
| From build, Up to build | A build range, both ends included |
| Locales | `en` matches `en-IN` |
| Share of installs | A percentage; an install stays in as the share grows |
| From, Until | A period in UTC |

A platform's own "must update below" and "asked to update below" builds apply to every install of it. Among flags of one name, the newest policy that applies wins.

## Links that open the app

| Path | Lists |
| --- | --- |
| `/.well-known/apple-app-site-association` | iOS platforms with a team ID, and the paths each opens |
| `/.well-known/assetlinks.json` | Android platforms with a signing fingerprint |

## Permissions

| Permission | Allows |
| --- | --- |
| `rainmill.mobile.manage` | Apps, platforms, policies and releases |
| `rainmill.mobile.devices` | The devices list |

## License

MIT or Apache-2.0, at your option.
