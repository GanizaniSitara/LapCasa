# Privacy

Last updated: November 8, 2026

## Overview

Lap is designed as a local-first photo manager. Your photo library is processed on your device, and your files remain under your control.

This document explains what data Lap accesses, what data Lap does not collect by default, and when network access may occur.

## What Lap Accesses

Lap may access the following data on your device in order to provide its features:

- Photos, videos, and folders that you choose to add to a library
- File metadata such as filenames, paths, timestamps, size, format, EXIF data, ratings, tags, comments, and rotation state
- Generated local app data such as thumbnails, previews, indexes, search data, embeddings, face clustering data, video indexing data, and other library-related cache or database records

Lap uses this data to support browsing, search, deduplication, tagging, ratings, file type filtering, video support, face clustering, and other library management features.

## Local Processing

Lap is intended to process your library locally on your device.

By default:

- Your photos and videos are not uploaded to a Lap cloud service
- AI search, smart tags, face detection, face clustering, thumbnail generation, RAW previews, and video indexing are processed locally
- Lap does not include advertising trackers
- Lap does not track daily active usage, session counts, or feature usage patterns
- The frontend does not record user behavior, clicks, or navigation events

## Network Access

Photo indexing and AI inference run locally. Some optional features contact external services:

Maps start in Offline/local mode, including after upgrading older provider settings. The bundled Natural Earth overview and a raster MBTiles file you select are read locally. Missing local coverage never triggers an online provider. The included overview does not contain street detail; local map files must be obtained under terms that permit their use.

- Opening external links such as the project website or GitHub repository in your browser
- Downloading optional model packs after you review and accept the displayed provider terms, or requesting the separate GPU add-on. The download provider and its infrastructure receive normal connection information such as your IP address; your photo library is not uploaded.
- Online maps request tiles from the selected provider: OpenStreetMap, Esri satellite imagery, or Tianditu when configured. Tile coordinates reveal the geographic area being viewed, and the provider receives your IP address and normal connection information. A Tianditu token is sent to Tianditu as part of its tile requests.
- Importing an image from a web URL contacts that URL and any redirect destination to download the image. The source receives your IP address and the requested URL.
- Opening a location in an external maps app passes its coordinates to that app, with a generic LapCasa label instead of the photo's filename. Review the external app's privacy terms before using it.
- Email, export and “Open in external app” actions pass the selected files to the application or destination you choose. Those applications have their own privacy and network behaviour. LapCasa does not automatically send email.

Downloading a model does not upload your photos, filenames, search terms, embeddings or library database. The download provider can identify which model file was requested; a local consent receipt is not sent to it.

## No Telemetry or Usage Statistics

Lap contains no telemetry, analytics, or tracking services. No call-home mechanisms, session tracking, crash reporters, or usage statistics exist in either the backend or frontend.

LapCasa does not automatically upload your photo library, search queries, embeddings or face clusters to a cloud service. The optional map and sharing actions described above can disclose locations or selected files. Local processing does not make those external actions private.

## Data Storage

Lap stores application data locally on your device. This may include:

- App settings
- Local databases for library records, generated thumbnails and previews, search indexes, embeddings, face clustering data, video indexing data, tags, ratings, comments, and related cache data

This local data is used to provide the app's functionality and improve performance on your device.

## Third-Party Services

LapCasa does not provide a cloud photo-storage or telemetry service. Optional downloads, online maps, external links and applications can contact third parties as described above.

## Changes to This Document

This document may be updated as the app evolves. Privacy-related changes should be reflected in the repository and user-facing documentation.

## Contact

For privacy-related questions or concerns, please open an issue in the project repository:

[LapCasa issues](https://github.com/GanizaniSitara/LapCasa/issues)
