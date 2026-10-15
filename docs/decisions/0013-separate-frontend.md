# 0013. Separate frontend

Status: Accepted (technology still open)

## Context
Institutions should be able to replace or restyle the frontend without touching the backend or the schema.

## Decision
- The frontend is a separate application that uses only the public API, like any other client.
- In production the frontend and the API are served on the same site through a reverse proxy, so cookie auth works without cross-site setup.
- The technology (React SPA or Leptos) is decided in a later ADR, after the API exists.

## Consequences
- The backend is developed and tested without a UI (HTTP files / CLI).
- The rich-text editor will most likely need JS whichever framework is chosen.
