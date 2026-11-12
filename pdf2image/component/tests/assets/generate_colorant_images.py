# SPDX-License-Identifier: MIT
"""Generate a synthetic scanned pattern and full-page colorant image overlays."""
from pathlib import Path


def generate():
    objects = [b"<< /Type /Catalog /Pages 2 0 R >>", b""]

    def add(body):
        objects.append(body)
        return len(objects)

    def stream(attributes, data):
        return add(f"<< /Length {len(data)} {attributes} >>\nstream\n".encode()
                   + data + b"\nendstream")

    scan = stream("/Type /XObject /Subtype /Image /Width 8 /Height 8 "
                  "/BitsPerComponent 8 /ColorSpace /DeviceGray",
                  bytes(255 if (x + y) % 2 else 0 for y in range(8) for x in range(8)))
    tint = stream("/FunctionType 4 /Domain [0 1 0 1] /Range [0 1]", b"{ pop }")
    separation = "[/Separation /{} /DeviceGray << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >>]"
    spaces = [None, separation.format("None"), separation.format("Spot"),
              f"[/DeviceN [/None /None] /DeviceGray {tint} 0 R]",
              f"[/DeviceN [/None /Spot] /DeviceGray {tint} 0 R]", "/DeviceGray"]
    kids = []
    for space in spaces:
        resources = f"/Scan {scan} 0 R"
        commands = "q 64 0 0 64 0 0 cm /Scan Do Q"
        if space is not None:
            overlay = stream("/Type /XObject /Subtype /Image /Width 1 /Height 1 "
                             f"/BitsPerComponent 8 /ColorSpace {space}",
                             bytes([128, 128]) if space.startswith("[/DeviceN") else bytes([128]))
            resources += f" /Overlay {overlay} 0 R"
            commands += " q 64 0 0 64 0 0 cm /Overlay Do Q"
        content = stream("", commands.encode())
        page = add(f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 64 64] "
                   f"/Resources << /XObject << {resources} >> >> /Contents {content} 0 R >>".encode())
        kids.append(f"{page} 0 R")
    objects[1] = f"<< /Type /Pages /Count {len(kids)} /Kids [{' '.join(kids)}] >>".encode()
    pdf, offsets = bytearray(b"%PDF-1.4\n"), []
    for number, body in enumerate(objects, 1):
        offsets.append(len(pdf))
        pdf.extend(f"{number} 0 obj\n".encode() + body + b"\nendobj\n")
    xref = len(pdf)
    pdf.extend(f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode())
    for offset in offsets:
        pdf.extend(f"{offset:010} 00000 n \n".encode())
    pdf.extend(f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
    return pdf


if __name__ == "__main__":
    Path(__file__).with_name("colorant-images.pdf").write_bytes(generate())
