<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/kicksideai/docs/main/getting-started/assets/brand/kickside-logo-dark.png">
  <img src="https://raw.githubusercontent.com/kicksideai/docs/main/getting-started/assets/brand/kickside-logo-light.png" alt="KickSide" width="320">
</picture>

# KickSide converters

Three WebAssembly document-processing modules with Lua SDKs:

| Module | Operation |
| --- | --- |
| [doc2md](doc2md) | Converts supported documents and archives to Markdown. |
| [html2pdf](html2pdf) | Renders HTML as PDF. |
| [pdf2image](pdf2image) | Renders PDF pages as images. |

The repository contains the WebAssembly component sources, Lua APIs, and unit-test harnesses. The modules run independently of a browser interface and support document processing in KickSide uploads and other modules.

Install `kickside/doc2md`, `kickside/html2pdf`, or `kickside/pdf2image` from the [Wippy Hub](https://hub.wippy.ai), then use the owning module's README for its SDK and input options.

License: [MIT](LICENSE). [Third-party notices](THIRD_PARTY_NOTICES.md) cover bundled libraries.

[Documentation](https://github.com/kicksideai/docs) · [KickSide](https://kickside.ai)
