export default {
  name: "wonder",
  version: "0.1.0",
  description: "Generate creative prompts, questions, or ideas about a topic. Useful for brainstorming, exploration, and creative thinking.",
  parameters: {
    type: "object",
    properties: {
      topic: { type: "string", description: "Topic to wonder about" },
      style: {
        type: "string",
        description: "Style of wondering: 'questions', 'ideas', 'connections'. Default: 'questions'",
        enum: ["questions", "ideas", "connections"]
      },
      count: { type: "integer", description: "Number of items to generate. Default: 5" }
    },
    required: ["topic"]
  },
  execute: function(input) {
    var style = input.style || "questions";
    var count = input.count || 5;

    var prompts = {
      questions: [
        "What if " + input.topic + " worked completely differently?",
        "Why does " + input.topic + " exist in its current form?",
        "What would happen if we removed " + input.topic + " entirely?",
        "How might " + input.topic + " evolve in the next decade?",
        "What hidden assumptions do we make about " + input.topic + "?",
        "Who benefits most from " + input.topic + "?",
        "What is the simplest version of " + input.topic + "?",
        "What would an alien think about " + input.topic + "?"
      ],
      ideas: [
        "Combine " + input.topic + " with something unexpected",
        "Apply " + input.topic + " to a completely different domain",
        "Simplify " + input.topic + " to its essence",
        "Scale " + input.topic + " up by 100x",
        "Make " + input.topic + " accessible to children",
        "Automate the hardest part of " + input.topic,
        "Turn " + input.topic + " into a game",
        "Find the opposite of " + input.topic
      ],
      connections: [
        input.topic + " relates to systems thinking through...",
        input.topic + " parallels patterns in nature like...",
        input.topic + " connects to music through...",
        input.topic + " mirrors challenges in architecture...",
        input.topic + " echoes themes from history...",
        input.topic + " intersects with philosophy at...",
        input.topic + " shares principles with cooking...",
        input.topic + " aligns with patterns in biology..."
      ]
    };

    var items = prompts[style] || prompts.questions;
    var selected = items.slice(0, count);

    var header = "Wondering about: " + input.topic + " (" + style + ")\n";
    var lines = [];
    for (var i = 0; i < selected.length; i++) {
      lines.push((i + 1) + ". " + selected[i]);
    }

    return header + "\n" + lines.join("\n");
  }
}
