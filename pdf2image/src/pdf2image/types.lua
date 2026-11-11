-- SPDX-License-Identifier: MIT
type RasterizeRef = {storage_id: string?, storage_path: string?, url: string?, bytes: string?}
type RasterizeOptions = {dpi: number?, format: string?, quality: number?, pages: {number} | string?}
type RasterizeRequest = RasterizeOptions & {ref: RasterizeRef, count_only: boolean?}
-- Adapter errors always include classification. Missing native retryability defaults
-- to true for Unavailable/Timeout, false for every other kind; missing code is the kind.
type RasterizeError = {message: string, kind: string, retryable: boolean, code: string}
type RasterizePage = {page: number?, bytes: string, format: string}
type RasterizeResult = {ok: {total: number, pages: {RasterizePage}?}?, err: RasterizeError?}
type PdfDocument = {pages: number, render: (RasterizeOptions) -> ({string}?, error?)}
return {}
