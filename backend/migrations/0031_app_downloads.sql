-- ADR-0064. Where the institution's desktop app can be downloaded, one link per kind of installer. An admin
-- sets them in the web app; the public profile lists them, and the web app shows a download page and a hint when
-- there is at least one. No row, no hint.
CREATE TABLE org.app_downloads (
    platform   text        PRIMARY KEY CHECK (platform IN ('windows', 'macos_arm', 'macos_intel', 'linux_deb', 'linux_appimage')),
    url        text        NOT NULL CHECK (char_length(url) BETWEEN 9 AND 2048),
    updated_at timestamptz NOT NULL DEFAULT now()
);
