import os
from pypdf import PdfReader

folder = r"C:\Users\physi\Desktop\Justin loan docs"
pdfs = ["2024 w2.pdf", "2025 w2.pdf", "2025 year end paystub.pdf", "paystub 1.pdf", "paystub 2.pdf"]

for name in pdfs:
    path = os.path.join(folder, name)
    try:
        reader = PdfReader(path)
        text = ""
        for page in reader.pages:
            text += page.extract_text() or ""
        # Print just first 500 chars of each
        print(f"\n=== {name} ===")
        print(text[:500])
        print("...")
    except Exception as e:
        print(f"\n=== {name} === ERROR: {e}")
