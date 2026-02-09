# Mini Markdown Parser

Implement a constrained Markdown-to-HTML renderer.

## API

```python
render_markdown(text: str) -> str
```

## CLI

`python -m src.markdown [path]`

- If `path` is provided, read from that file.
- Otherwise read from stdin.
- Write rendered HTML to stdout.

## Supported syntax

### Block-level

1. Headings:
   - `# ` -> `<h1>...</h1>`
   - `## ` -> `<h2>...</h2>`
   - `### ` -> `<h3>...</h3>`
2. Paragraphs: separated by blank lines
3. Unordered lists: consecutive `- ` lines form `<ul> ... </ul>`
4. Fenced code blocks:
   - start and end with triple backticks
   - output: `<pre><code>...</code></pre>`

### Inline

Inside paragraph and list item text:

- `**bold**` -> `<strong>bold</strong>`
- `*italic*` -> `<em>italic</em>`
- `` `code` `` -> `<code>code</code>`
- `[text](url)` -> `<a href="url">text</a>`

## Escaping

Escape raw text nodes:

- `&` -> `&amp;`
- `<` -> `&lt;`
- `>` -> `&gt;`

Do not escape generated HTML tags.

## Determinism

Output must be deterministic and exactly match expected test strings.
