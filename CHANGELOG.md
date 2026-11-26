# Changelog

Notable changes to `rainmill-mobile`. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/) as Cargo reads it: before 1.0 a minor
bump is breaking and a patch is additive.

## [Unreleased]

### Added

- Target dimensions: an application registers a `TargetDimension` (`add_target_dimension`), a policy names it under `dimensions`, and an app sends its value in the check-in's `context`. A policy that is on applies only where each dimension allows it; one that is off is never put to them.
- First cut. Apps with Android, iOS and web platforms, a check-in endpoint answering with flags, maintenance, updates and banners, release records, a devices list, and the association files that let links open the app.
- A Google Play credential with a Preflight that tries the key, the Developer API and access to the app, and a publishing guide under Apps.
- Build files on a release, in the media library or attached to the release alone as each app chooses, added from the admin or by a pipeline with the app's upload token at `POST /mobile/v1/releases`.
