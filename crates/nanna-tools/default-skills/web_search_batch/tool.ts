export default {
  name: "web_search_batch",
  description: "Search the web for multiple queries at once. Returns grouped results for each query. Useful for research tasks requiring multiple searches.",
  parameters: {
    type: "object",
    properties: {
      queries: {
        type: "array",
        description: "Array of search queries (max 5)"
      },
      results_per_query: { type: "integer", description: "Number of results per query. Default: 3" }
    },
    required: ["queries"]
  },
  execute: function(input) {
    var perQuery = input.results_per_query || 3;
    var limitedQueries = input.queries.slice(0, 5);

    var apiKey = Nanna.getEnv("BRAVE_API_KEY");
    if (!apiKey) {
      return "Error: BRAVE_API_KEY not set. Configure it in your environment or nanna config.";
    }

    var sections = [];

    for (var qi = 0; qi < limitedQueries.length; qi++) {
      var query = limitedQueries[qi];
      var url = "https://api.search.brave.com/res/v1/web/search?q=" + encodeURIComponent(query) + "&count=" + perQuery;
      var results;

      try {
        var response = Nanna.fetch(url, {
          headers: {
            "Accept": "application/json",
            "Accept-Encoding": "gzip",
            "X-Subscription-Token": apiKey
          }
        });

        if (response.status !== 200) {
          sections.push("=== Query: \"" + query + "\" ===\nError: HTTP " + response.status);
          continue;
        }

        var data = JSON.parse(response.body);
        results = (data.web && data.web.results) || [];
      } catch (e) {
        sections.push("=== Query: \"" + query + "\" ===\nError: " + (e.message || "Request failed"));
        continue;
      }

      if (results.length === 0) {
        sections.push("=== Query: \"" + query + "\" ===\nNo results found");
        continue;
      }

      var formatted = [];
      for (var i = 0; i < results.length; i++) {
        var r = results[i];
        var desc = r.description || "(no description)";
        formatted.push("  " + (i + 1) + ". " + r.title + "\n     " + r.url + "\n     " + desc);
      }

      sections.push("=== Query: \"" + query + "\" ===\n" + formatted.join("\n\n"));
    }

    return sections.join("\n\n");
  }
}
