export default {
  name: "read_pdf",
  description: "Extract text from a PDF file. Returns the text content of the document.",
  parameters: {
    type: "object",
    properties: {
      path: { type: "string", description: "Path to the PDF file" },
      pages: { type: "string", description: "Page range to extract (e.g. '1-5', '3'). Default: all pages" }
    },
    required: ["path"]
  },
  execute: function(input) {
    try {
      var result = Nanna.service("pdf.read", {
        path: input.path,
        pages: input.pages
      });
      var text = result.text || "(empty document)";
      var pageInfo = result.page_count ? " (" + result.page_count + " pages)" : "";
      return "PDF: " + input.path + pageInfo + "\n\n" + text;
    } catch (e) {
      return "Error: PDF reading service not available. " + e;
    }
  }
}
