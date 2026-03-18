import importlib
for mod in ['PyPDF2', 'pypdf', 'pdfplumber', 'fitz', 'pdfminer']:
    try:
        importlib.import_module(mod)
        print(f"{mod}: available")
    except ImportError:
        print(f"{mod}: not found")
