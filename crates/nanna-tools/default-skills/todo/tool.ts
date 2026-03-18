export default {
  name: "todo",
  description: "Manage a task checklist to track progress on multi-step work. Use this to keep track of what you need to do, what you're working on, and what's done. Helps maintain focus during complex tasks.",
  output: "context",
  parameters: {
    type: "object",
    properties: {
      action: {
        type: "string",
        description: "Action to perform: 'add', 'done', 'update', 'remove', 'clear', or 'list'. Default: 'list'",
        enum: ["add", "done", "update", "remove", "clear", "list"]
      },
      id: {
        type: "integer",
        description: "Task ID (for done/update/remove actions)"
      },
      text: {
        type: "string",
        description: "Task description (for add action) or status update (for update action)"
      },
      status: {
        type: "string",
        description: "Task status for update action: 'pending', 'in_progress', 'blocked', 'done'",
        enum: ["pending", "in_progress", "blocked", "done"]
      }
    },
    required: []
  },
  execute: function(input) {
    // Use a file in the working directory or a temp location
    var todoFile = ".nanna-todo.json";
    var todos = [];

    // Load existing todos
    try {
      var content = Nanna.readFile(todoFile);
      if (content) {
        todos = JSON.parse(content);
      }
    } catch (e) {
      // File doesn't exist yet, start fresh
      todos = [];
    }

    var action = input.action || "list";
    var nextId = 1;
    for (var i = 0; i < todos.length; i++) {
      if (todos[i].id >= nextId) {
        nextId = todos[i].id + 1;
      }
    }

    switch (action) {
      case "add": {
        if (!input.text) {
          return "Error: 'text' is required for add action";
        }
        var newTodo = {
          id: nextId,
          text: input.text,
          status: "pending",
          created: new Date().toISOString()
        };
        todos.push(newTodo);
        Nanna.writeFile(todoFile, JSON.stringify(todos, null, 2));
        return "Added task #" + newTodo.id + ": " + newTodo.text + "\n\n" + formatTodos(todos);
      }

      case "done": {
        if (!input.id) {
          return "Error: 'id' is required for done action";
        }
        var found = false;
        for (var i = 0; i < todos.length; i++) {
          if (todos[i].id === input.id) {
            todos[i].status = "done";
            found = true;
            break;
          }
        }
        if (!found) {
          return "Error: task #" + input.id + " not found";
        }
        Nanna.writeFile(todoFile, JSON.stringify(todos, null, 2));
        return "Marked task #" + input.id + " as done.\n\n" + formatTodos(todos);
      }

      case "update": {
        if (!input.id) {
          return "Error: 'id' is required for update action";
        }
        var found = false;
        for (var i = 0; i < todos.length; i++) {
          if (todos[i].id === input.id) {
            if (input.status) {
              todos[i].status = input.status;
            }
            if (input.text) {
              todos[i].text = input.text;
            }
            found = true;
            break;
          }
        }
        if (!found) {
          return "Error: task #" + input.id + " not found";
        }
        Nanna.writeFile(todoFile, JSON.stringify(todos, null, 2));
        return "Updated task #" + input.id + ".\n\n" + formatTodos(todos);
      }

      case "remove": {
        if (!input.id) {
          return "Error: 'id' is required for remove action";
        }
        var newTodos = [];
        var removed = false;
        for (var i = 0; i < todos.length; i++) {
          if (todos[i].id === input.id) {
            removed = true;
          } else {
            newTodos.push(todos[i]);
          }
        }
        if (!removed) {
          return "Error: task #" + input.id + " not found";
        }
        todos = newTodos;
        Nanna.writeFile(todoFile, JSON.stringify(todos, null, 2));
        return "Removed task #" + input.id + ".\n\n" + formatTodos(todos);
      }

      case "clear": {
        var doneCount = 0;
        var newTodos = [];
        for (var i = 0; i < todos.length; i++) {
          if (todos[i].status === "done") {
            doneCount++;
          } else {
            newTodos.push(todos[i]);
          }
        }
        todos = newTodos;
        Nanna.writeFile(todoFile, JSON.stringify(todos, null, 2));
        return "Cleared " + doneCount + " completed tasks.\n\n" + formatTodos(todos);
      }

      case "list":
      default:
        if (todos.length === 0) {
          return "No tasks. Use todo(action='add', text='...') to add one.";
        }
        return formatTodos(todos);
    }
  }
}

function formatTodos(todos) {
  if (todos.length === 0) {
    return "📋 No tasks.";
  }

  var statusIcon = {
    "pending": "⬜",
    "in_progress": "🔄",
    "blocked": "🚫",
    "done": "✅"
  };

  var lines = ["📋 Task List:"];
  var pending = [];
  var inProgress = [];
  var blocked = [];
  var done = [];

  for (var i = 0; i < todos.length; i++) {
    var t = todos[i];
    var icon = statusIcon[t.status] || "⬜";
    var line = "  " + icon + " #" + t.id + " " + t.text;
    if (t.status === "done") {
      done.push(line);
    } else if (t.status === "in_progress") {
      inProgress.push(line);
    } else if (t.status === "blocked") {
      blocked.push(line);
    } else {
      pending.push(line);
    }
  }

  if (inProgress.length > 0) {
    lines.push("In Progress:");
    lines = lines.concat(inProgress);
  }
  if (blocked.length > 0) {
    lines.push("Blocked:");
    lines = lines.concat(blocked);
  }
  if (pending.length > 0) {
    lines.push("Pending:");
    lines = lines.concat(pending);
  }
  if (done.length > 0) {
    lines.push("Done:");
    lines = lines.concat(done);
  }

  var total = todos.length;
  var doneCount = done.length;
  lines.push("");
  lines.push("Progress: " + doneCount + "/" + total + " tasks complete");

  return lines.join("\n");
}
