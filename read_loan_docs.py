import pypdf
import os
import glob

folder = r"C:\Users\physi\Desktop\Justin loan docs"
pdfs = glob.glob(os.path.join(folder, "*.pdf"))

for pdf_path in sorted(pdfs):
    print(f"\n{'='*60}")
    print(f"FILE: {os.path.basename(pdf_path)}")
    print(f"{'='*60}")
    reader = pypdf.PdfReader(pdf_path)
    for i, page in enumerate(reader.pages):
        text = page.extract_text()
        if text:
            print(text)
        else:
            print(f"[Page {i+1}: No extractable text - may be scanned image]")
