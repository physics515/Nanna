export default {
  name: "web_search",
  description: "Search the web using Brave Search API. Returns titles, URLs, and descriptions of search results.",
  parameters: {
    type: "object",
    properties: {
      query: { type: "string", description: "Search query" },
      count: { type: "integer", description: "Number of results to return. Default: 5" }
    },
    required: ["query"]
  },
  execute: function(input) {
    var numResults = input.count || 5;

    var apiKey = Nanna.getEnv("BRAVE_API_KEY");
    if (!apiKey) {
      return "Error: BRAVE_API_KEY not set. Configure it in your environment or nanna config.";
    }

    var url = "https://api.search.brave.com/res/v1/web/search?q=" + encodeURIComponent(input.query) + "&count=" + numResults;
    var response = Nanna.fetch(url, {
      headers: {
        "Accept": "application/json",
        "Accept-Encoding": "gzip",
        "X-Subscription-Token": apiKey
      }
    });

    if (response.status !== 200) {
      return "Error: Brave Search API returned status " + response.status + ": " + response.body.substring(0, 200);
    }

    var data;
    try {
      data = JSON.parse(response.body);
    } catch (e) {
      return "Error: Failed to parse search results";
    }

    var results = (data.web && data.web.results) || [];
    if (results.length === 0) {
      return "No results found for \"" + input.query + "\"";
    }

    var formatted = [];
    for (var i = 0; i < results.length; i++) {
      var r = results[i];
      var desc = r.description || "(no description)";
      formatted.push((i + 1) + ". " + r.title + "\n   " + r.url + "\n   " + desc);
    }

    return "Search results for \"" + input.query + "\":\n\n" + formatted.join("\n\n");
  }
}
