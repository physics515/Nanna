import os
from pypdf import PdfReader

folder = r"C:\Users\physi\Desktop\Justin loan docs"
output = r"D:\Development\nanna\pdf_output.txt"

pdfs = ["2024 w2.pdf", "2025 w2.pdf", "2025 year end paystub.pdf", "paystub 1.pdf", "paystub 2.pdf"]

with open(output, "w", encoding="utf-8") as out:
    for name in pdfs:
        path = os.path.join(folder, name)
        out.write(f"\n{'='*60}\n{name}\n{'='*60}\n")
        try:
            reader = PdfReader(path)
            for i, page in enumerate(reader.pages):
                text = page.extract_text()
                out.write(f"\n--- Page {i+1} ---\n{text}\n")
        except Exception as e:
            out.write(f"ERROR: {e}\n")

print("Done")
