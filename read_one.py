import sys
from pypdf import PdfReader

path = sys.argv[1]
reader = PdfReader(path)
for page in reader.pages:
    text = page.extract_text()
    if text:
        print(text)
