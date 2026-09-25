# GPT-6 Model Icon Search

Checked on 2026-09-25.

## Finding

Using the exact model IDs as the required icon names, the Iconify public search index returned no matches for `gpt-6-astra`, `gpt-6-sol`, or `gpt-6-luna`. The same nine searches across Iconify's primary API and two documented backup APIs returned zero results. Short names such as `astra` or `luna`, and icons chosen only for semantic similarity, do not meet this requirement.

Iconify documents its search endpoint as a way to search its hosted icon sets. The exact queries are directly reproducible:

- [gpt-6-astra on api.iconify.design](https://api.iconify.design/search?query=gpt-6-astra&limit=999), [api.simplesvg.com](https://api.simplesvg.com/search?query=gpt-6-astra&limit=999), [api.unisvg.com](https://api.unisvg.com/search?query=gpt-6-astra&limit=999)
- [gpt-6-sol on api.iconify.design](https://api.iconify.design/search?query=gpt-6-sol&limit=999), [api.simplesvg.com](https://api.simplesvg.com/search?query=gpt-6-sol&limit=999), [api.unisvg.com](https://api.unisvg.com/search?query=gpt-6-sol&limit=999)
- [gpt-6-luna on api.iconify.design](https://api.iconify.design/search?query=gpt-6-luna&limit=999), [api.simplesvg.com](https://api.simplesvg.com/search?query=gpt-6-luna&limit=999), [api.unisvg.com](https://api.unisvg.com/search?query=gpt-6-luna&limit=999)
- [Iconify search documentation](https://iconify.design/docs/api/queries.html#search-icons) and [public API overview](https://github.com/iconify/website/blob/main/docs/api/index.md).

The repository's installed web dependencies also contain no file named for these full IDs. These checks do not cover every unindexed repository, private icon collection, or asset marketplace, so the conclusion is limited to the sources searched.

## Official Assets

OpenAI's model pages identify [GPT-6 Astra](https://developers.openai.com/api/docs/models/gpt-6-astra), [GPT-6 Sol](https://developers.openai.com/api/docs/models/gpt-6-sol), and [GPT-6 Luna](https://developers.openai.com/api/docs/models/gpt-6-luna) with model-specific images. Their first-party image URLs are [gpt-6-astra.png](https://developers.openai.com/images/api/models/icons/gpt-6-astra.png), [gpt-6-sol.png](https://developers.openai.com/images/api/models/icons/gpt-6-sol.png), and [gpt-6-luna.png](https://developers.openai.com/images/api/models/icons/gpt-6-luna.png).

The checked images are 128 by 128 PNGs. They are square artwork containing the numeral 6 and the model name, rather than standalone glyphs. At 16 by 16 pixels, their text and visual differences are not reliably legible. I found no per-model HEX palette or separate small-glyph asset in the official model pages and image resources checked here; colors sampled from the artwork would be derived values, not official identity tokens.

## UI Decision

There is no verified third-party candidate in the searched index with an exact full-ID name. The UI therefore uses an owner-approved fallback set of general-purpose MDI symbols: `creation` for Astra, `weather-sunny` for Sol, and `moon-waning-crescent` for Luna. These are not official OpenAI icons or model-specific artwork; the product assigns them as generic identity glyphs and differentiates each with product-defined theme colors. The GPT-6 identities retain their model IDs in accessible names and tooltips.
