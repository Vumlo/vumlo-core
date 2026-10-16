# 0027. Document types live in api-types; video blocks store a provider and an id; an option is one block

Status: Accepted
Amends: ADR-0025

## Context
ADR-0025 fixed the block document format. Building it raised three points that it left open or got wrong: where the Rust types of the format live, how a video block refers to its video, and how big an answer option is.

## Options
Types: (a) in `content`, with the API showing "any object": clients are generated from OpenAPI (ADR-0012) and would have to describe the format by hand; (b) **in `api-types`, with `content` using them directly**: the format is both the wire format and the storage format, so there is nothing to map.

Video: (a) a URL on a host allowlist: a pasted `watch` link is not an embed address, and the address of an `iframe` would come from user input; (b) **a provider and an id**, with the embed address built by the client from a fixed template.

Option: (a) any document under the `option` profile; (b) **exactly one block**.

## Decision
- The types of the block document live in `api-types`. `content` validates and stores them and may use them in its services; the rule that DTOs stay out of services does not apply to them. A request carries the document as a JSON value so that validation can report a path into it; the OpenAPI schema is the typed one.
- A `video` block has `props: {"provider": "youtube" | "vimeo", "video_id": string}`. A YouTube id is 11 characters from `[A-Za-z0-9_-]`; a Vimeo id is 1 to 12 digits. The server stores no URL for a video. The client builds the embed address from a fixed template (`https://www.youtube-nocookie.com/embed/<id>`, `https://player.vimeo.com/video/<id>`), and the editor turns a pasted link into a provider and an id. An uploaded video (a later task) is another variant with a `file_id`.
- `editor.video_providers` (default `["youtube", "vimeo"]`) replaces `editor.video_hosts` of ADR-0025. It can only narrow.
- The `option` profile allows exactly one block, without children: a paragraph, a code block or a math block (an image once images exist).
- Everything else in ADR-0025 stands: the features and profiles, the three layers of limits, the limits.

## Consequences
- Clients get real types for documents from the generated OpenAPI.
- A `link` item's `embed` (ADR-0024) shows a video inline only when its url is a recognizable `youtube` or `vimeo` link of a provider that `editor.video_providers` lists.
- A document with a video block never holds a URL that the server or the browser could be tricked into loading from user input.
