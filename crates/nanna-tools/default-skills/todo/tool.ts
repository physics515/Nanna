export default {
  name: "todo",
  version: "0.2.0",
  description: "Agent-grade task store. Track and drive multi-step work: 'next' returns the ONE actionable task (unblocked, highest priority), 'add' creates tasks (with parent_id for subtasks, depends_on for ordering, acceptance for a machine-checkable done condition), 'done' completes a task (its acceptance check is verified by the harness first), 'note' saves findings for future steps, 'query' filters (e.g. 'p1 & !done', '@label', 'overdue'). Tasks persist across sessions via scope: session (default), workspace, or global.",
  output: "context",
  parameters: {
    type: "object",
    properties: {
      action: {
        type: "string",
        description: "Action: 'next' (the one actionable task), 'add', 'update', 'done', 'note', 'query', 'list'. Legacy: 'create', 'remove', 'clear', 'clear_all'. Default: 'list'",
        enum: ["next", "add", "update", "done", "note", "query", "list", "create", "remove", "clear", "clear_all"]
      },
      id: { type: "integer", description: "Task id (for update/done/note/remove)" },
      title: { type: "string", description: "Task title (for add/update). 'text' also accepted" },
      text: { type: "string", description: "Alias for title, or note content" },
      description: { type: "string", description: "Longer task description" },
      parent_id: { type: "integer", description: "Parent task id — makes this a subtask (decomposition)" },
      priority: { type: "integer", description: "1 (highest) to 4 (lowest). Default 3" },
      labels: { type: "array", items: { type: "string" }, description: "Labels, queryable as @label" },
      tools: { type: "array", items: { type: "string" }, description: "Tool names this task needs (scopes the agent's active tools during harness runs)" },
      due_at: { type: "string", description: "Due date, ISO format YYYY-MM-DD" },
      recurrence: { type: "string", description: "Cron expression (5 fields) — task reopens on schedule" },
      depends_on: { type: "array", items: { type: "integer" }, description: "Task ids that must complete first. Blocked status is derived from this" },
      acceptance: {
        type: "object",
        description: "Machine-checkable done condition, verified by the harness: {kind:'command', command:'...'} (exit 0), {kind:'file_exists', path:'...'}, or {kind:'regex', pattern:'...', path|command:'...'}"
      },
      content: { type: "string", description: "Note content (for note action)" },
      filter: { type: "string", description: "Query filter: & | ! (), p1..p4, @label, #project, overdue, today, no date, due before: DATE, search: text, subtask, blocked, done, pending, in_progress" },
      scope: { type: "string", description: "Task scope: 'session' (default), 'workspace', or 'global'", enum: ["session", "workspace", "global"] },
      project: { type: "string", description: "Project name, queryable as #project" },
      assignee: { type: "string", description: "Which agent owns this task" },
      status: {
        type: "string",
        description: "For update: 'pending' or 'in_progress' ('done' must go through the done action; 'blocked' is derived from depends_on)",
        enum: ["pending", "in_progress", "blocked", "done"]
      },
      items: { type: "string", description: "JSON array of task titles for legacy 'create' (replaces the session's tasks)" }
    },
    required: []
  },
  execute: function(input) {
    var action = input.action || "list";
    var sessionId = Nanna.sessionId();
    var scope = input.scope || "session";

    // Route v0.1 status shortcuts through the right v0.2 paths.
    if (action === "update" && input.status === "done") {
      action = "done";
    }

    try {
      migrateLegacyFile(sessionId);
      return serviceExecute(action, input, sessionId, scope);
    } catch (e) {
      var msg = "" + (e && e.message ? e.message : e);
      if (msg.indexOf("Service not found") !== -1) {
        // No daemon task store (legacy/embedded context) — v0.1 file behavior.
        return legacyExecute(action, input, sessionId);
      }
      return "Error: " + msg;
    }
  }
}

// ---------------------------------------------------------------------------
// v0.2: Turso-backed via Nanna.service("tasks.*")
// ---------------------------------------------------------------------------

function serviceExecute(action, input, sessionId, scope) {
  var base = { scope: scope, session_id: sessionId };

  switch (action) {
    case "next": {
      var res = Nanna.service("tasks.next", base);
      if (!res.task) {
        return "No actionable tasks. Everything is done, blocked-free, or the list is empty.";
      }
      return { content: formatNext(res.task), data: { task: res.task } };
    }

    case "add": {
      var title = input.title || input.text;
      if (!title) { return "Error: 'title' is required for add"; }
      var params = compact({
        scope: scope, session_id: sessionId, title: title,
        description: input.description, parent_id: input.parent_id,
        priority: input.priority, labels: input.labels, tools: input.tools,
        due_at: input.due_at, recurrence: input.recurrence,
        depends_on: input.depends_on, acceptance: input.acceptance,
        project: input.project, assignee: input.assignee
      });
      var res = Nanna.service("tasks.add", params);
      return "Added task #" + res.task.id + ": " + res.task.title + "\n\n" + listSummary(base);
    }

    case "update": {
      if (!input.id) { return "Error: 'id' is required for update"; }
      var params = compact({
        id: input.id, title: input.title, text: input.text,
        description: input.description, status: input.status,
        priority: input.priority, labels: input.labels, tools: input.tools,
        due_at: input.due_at, recurrence: input.recurrence,
        depends_on: input.depends_on, acceptance: input.acceptance,
        project: input.project, assignee: input.assignee,
        parent_id: input.parent_id
      });
      var res = Nanna.service("tasks.update", params);
      return "Updated task #" + res.task.id + ".\n\n" + listSummary(base);
    }

    case "done": {
      if (!input.id) { return "Error: 'id' is required for done"; }
      var res = Nanna.service("tasks.done", {
        id: input.id, workdir: Nanna.workdir(), actor: "todo-tool"
      });
      if (!res.done) {
        return "Task #" + input.id + " is NOT done — acceptance check failed: " + res.verdict;
      }
      var extra = "";
      if (res.auto_completed && res.auto_completed.length > 0) {
        extra = " Parent task(s) auto-completed: #" + res.auto_completed.join(", #") + ".";
      }
      return "Marked task #" + input.id + " as done." + extra + "\n\n" + listSummary(base);
    }

    case "note": {
      if (!input.id) { return "Error: 'id' is required for note"; }
      var content = input.content || input.text;
      if (!content) { return "Error: 'content' is required for note"; }
      Nanna.service("tasks.note", { id: input.id, content: content, author: "agent" });
      return "Note saved on task #" + input.id + ".";
    }

    case "query": {
      if (!input.filter) { return "Error: 'filter' is required for query"; }
      var res = Nanna.service("tasks.query", {
        scope: scope, session_id: sessionId, filter: input.filter
      });
      if (res.tasks.length === 0) { return "No tasks match: " + input.filter; }
      return formatTaskList(res.tasks, "Tasks matching '" + input.filter + "':");
    }

    case "remove": {
      if (!input.id) { return "Error: 'id' is required for remove"; }
      var res = Nanna.service("tasks.remove", { id: input.id });
      return "Removed " + res.removed + " task(s).\n\n" + listSummary(base);
    }

    case "create": {
      // Legacy v0.1: replace the session's tasks with a fresh list.
      var itemsList = [];
      if (!input.items && input.tasks) { input.items = input.tasks; }
      if (input.items) {
        try { itemsList = JSON.parse(input.items); }
        catch (e2) {
          itemsList = input.items.split("\n").filter(function(s) { return s.trim().length > 0; });
        }
      } else if (input.text || input.title) {
        itemsList = [input.text || input.title];
      } else {
        return "Error: 'items' (JSON array) or 'title' required for create";
      }
      Nanna.service("tasks.clear", { scope: scope, session_id: sessionId, closed_only: false });
      for (var i = 0; i < itemsList.length; i++) {
        Nanna.service("tasks.add", {
          scope: scope, session_id: sessionId, title: "" + itemsList[i]
        });
      }
      return "Created " + itemsList.length + " tasks.\n\n" + listSummary(base);
    }

    case "clear": {
      var res = Nanna.service("tasks.clear", {
        scope: scope, session_id: sessionId, closed_only: true
      });
      return "Cleared " + res.removed + " completed tasks.\n\n" + listSummary(base);
    }

    case "clear_all": {
      var res = Nanna.service("tasks.clear", {
        scope: scope, session_id: sessionId, closed_only: false
      });
      return "Cleared all " + res.removed + " tasks.";
    }

    case "list":
    default: {
      var res = Nanna.service("tasks.list", {
        scope: scope, session_id: sessionId, include_done: true
      });
      if (res.tasks.length === 0) {
        return "No tasks. Use todo(action='add', title='...') to add one.";
      }
      return formatTaskList(res.tasks, "📋 Task List:");
    }
  }
}

function listSummary(base) {
  var counts = Nanna.service("tasks.counts", base);
  return "Progress: " + counts.closed + " done, " + counts.open + " open.";
}

// Drop undefined/null members: the bridge serializes `undefined` as null,
// and a partial update must not look like a request to clear fields.
function compact(obj) {
  var out = {};
  for (var key in obj) {
    if (obj.hasOwnProperty(key) && obj[key] !== undefined && obj[key] !== null) {
      out[key] = obj[key];
    }
  }
  return out;
}

function formatNext(task) {
  var lines = ["▶ Next task #" + task.id + ": " + task.title];
  if (task.description) { lines.push(task.description); }
  if (task.acceptance) {
    lines.push("Done when: " + describeAcceptance(task.acceptance));
  }
  if (task.tools && task.tools.length > 0) {
    lines.push("Tools: " + task.tools.join(", "));
  }
  if (task.notes && task.notes.length > 0) {
    lines.push("Notes:");
    for (var i = 0; i < task.notes.length; i++) {
      lines.push("- " + task.notes[i]);
    }
  }
  return lines.join("\n");
}

function describeAcceptance(a) {
  if (a.kind === "command") { return "`" + a.command + "` exits 0"; }
  if (a.kind === "file_exists") { return "file `" + a.path + "` exists"; }
  if (a.kind === "regex") {
    return "/" + a.pattern + "/ matches " + (a.path ? ("`" + a.path + "`") : ("output of `" + a.command + "`"));
  }
  return JSON.stringify(a);
}

function formatTaskList(tasks, header) {
  var statusIcon = {
    "pending": "⬜",
    "in_progress": "🔄",
    "done": "✅",
    "cancelled": "🚫"
  };
  var lines = [header];
  var done = 0;
  for (var i = 0; i < tasks.length; i++) {
    var t = tasks[i];
    var icon = t.blocked && t.status !== "done" ? "⛔" : (statusIcon[t.status] || "⬜");
    var indent = t.parent_id ? "    " : "  ";
    var line = indent + icon + " #" + t.id + " " + t.title;
    if (t.priority && t.priority !== 3) { line += " (p" + t.priority + ")"; }
    if (t.blocked && t.status !== "done") { line += " [blocked by #" + t.depends_on.join(", #") + "]"; }
    if (t.due_at) { line += " (due " + t.due_at + ")"; }
    lines.push(line);
    if (t.status === "done") { done++; }
  }
  lines.push("");
  lines.push("Progress: " + done + "/" + tasks.length + " tasks complete");
  return lines.join("\n");
}

// ---------------------------------------------------------------------------
// One-time migration of the v0.1 per-session JSON file
// ---------------------------------------------------------------------------

function migrateLegacyFile(sessionId) {
  if (!sessionId) { return; }
  var todoFile = ".nanna-todo-" + sessionId + ".json";
  var content = null;
  try { content = Nanna.readFile(todoFile); } catch (e) { return; }
  if (!content) { return; }
  var parsed = null;
  try { parsed = JSON.parse(content); } catch (e) { return; }
  if (!parsed || !parsed.length) {
    // Empty or already-migrated ({"migrated":true} has no .length)
    return;
  }
  var items = [];
  for (var i = 0; i < parsed.length; i++) {
    items.push({ text: "" + parsed[i].text, status: "" + (parsed[i].status || "pending") });
  }
  try {
    Nanna.service("tasks.import", { session_id: sessionId, items: items });
    Nanna.writeFile(todoFile, JSON.stringify({ migrated: true, at: new Date().toISOString() }));
    Nanna.log("info", "Migrated " + items.length + " v0.1 todo items into the task store");
  } catch (e) {
    // Service missing — leave the file for the legacy path.
  }
}

// ---------------------------------------------------------------------------
// v0.1 fallback: per-session JSON file (used when the daemon task store
// services are unavailable)
// ---------------------------------------------------------------------------

function legacyExecute(action, input, sessionId) {
  var todoFile = sessionId ? ".nanna-todo-" + sessionId + ".json" : ".nanna-todo.json";
  var todos = [];
  try {
    var content = Nanna.readFile(todoFile);
    if (content) {
      var parsed = JSON.parse(content);
      if (parsed && parsed.length !== undefined) { todos = parsed; }
    }
  } catch (e) {
    todos = [];
  }

  var nextId = 1;
  for (var i = 0; i < todos.length; i++) {
    if (todos[i].id >= nextId) { nextId = todos[i].id + 1; }
  }

  switch (action) {
    case "next": {
      for (var i = 0; i < todos.length; i++) {
        if (todos[i].status === "in_progress") {
          return "▶ Next task #" + todos[i].id + ": " + todos[i].text;
        }
      }
      for (var i = 0; i < todos.length; i++) {
        if (todos[i].status === "pending") {
          return "▶ Next task #" + todos[i].id + ": " + todos[i].text;
        }
      }
      return "No actionable tasks.";
    }

    case "create": {
      var itemsList = [];
      if (!input.items && input.tasks) { input.items = input.tasks; }
      if (input.items) {
        try { itemsList = JSON.parse(input.items); }
        catch (e) {
          itemsList = input.items.split("\n").filter(function(s) { return s.trim().length > 0; });
        }
      } else if (input.text || input.title) {
        itemsList = [input.text || input.title];
      } else {
        return "Error: 'items' (JSON array) or 'title' required for create action";
      }
      todos = [];
      for (var i = 0; i < itemsList.length; i++) {
        todos.push({ id: i + 1, text: itemsList[i], status: "pending", created: new Date().toISOString() });
      }
      Nanna.writeFile(todoFile, JSON.stringify(todos, null, 2));
      return "Created " + todos.length + " tasks.\n\n" + legacyFormat(todos);
    }

    case "add": {
      var title = input.title || input.text;
      if (!title) { return "Error: 'title' is required for add action"; }
      var newTodo = { id: nextId, text: title, status: "pending", created: new Date().toISOString() };
      todos.push(newTodo);
      Nanna.writeFile(todoFile, JSON.stringify(todos, null, 2));
      return "Added task #" + newTodo.id + ": " + newTodo.text + "\n\n" + legacyFormat(todos);
    }

    case "done": {
      if (!input.id) { return "Error: 'id' is required for done action"; }
      var found = false;
      for (var i = 0; i < todos.length; i++) {
        if (todos[i].id === input.id) { todos[i].status = "done"; found = true; break; }
      }
      if (!found) { return "Error: task #" + input.id + " not found"; }
      Nanna.writeFile(todoFile, JSON.stringify(todos, null, 2));
      return "Marked task #" + input.id + " as done.\n\n" + legacyFormat(todos);
    }

    case "update": {
      if (!input.id) { return "Error: 'id' is required for update action"; }
      var found = false;
      for (var i = 0; i < todos.length; i++) {
        if (todos[i].id === input.id) {
          if (input.status) { todos[i].status = input.status; }
          if (input.text || input.title) { todos[i].text = input.text || input.title; }
          found = true;
          break;
        }
      }
      if (!found) { return "Error: task #" + input.id + " not found"; }
      Nanna.writeFile(todoFile, JSON.stringify(todos, null, 2));
      return "Updated task #" + input.id + ".\n\n" + legacyFormat(todos);
    }

    case "remove": {
      if (!input.id) { return "Error: 'id' is required for remove action"; }
      var newTodos = [];
      var removed = false;
      for (var i = 0; i < todos.length; i++) {
        if (todos[i].id === input.id) { removed = true; } else { newTodos.push(todos[i]); }
      }
      if (!removed) { return "Error: task #" + input.id + " not found"; }
      Nanna.writeFile(todoFile, JSON.stringify(newTodos, null, 2));
      return "Removed task #" + input.id + ".\n\n" + legacyFormat(newTodos);
    }

    case "clear": {
      var doneCount = 0;
      var newTodos = [];
      for (var i = 0; i < todos.length; i++) {
        if (todos[i].status === "done") { doneCount++; } else { newTodos.push(todos[i]); }
      }
      Nanna.writeFile(todoFile, JSON.stringify(newTodos, null, 2));
      return "Cleared " + doneCount + " completed tasks.\n\n" + legacyFormat(newTodos);
    }

    case "clear_all": {
      var totalCount = todos.length;
      Nanna.writeFile(todoFile, JSON.stringify([], null, 2));
      return "Cleared all " + totalCount + " tasks.\n\n📋 No tasks.";
    }

    case "note":
    case "query":
      return "Error: '" + action + "' requires the daemon task store (not available here)";

    case "list":
    default:
      if (todos.length === 0) {
        return "No tasks. Use todo(action='add', title='...') to add one.";
      }
      return legacyFormat(todos);
  }
}

function legacyFormat(todos) {
  if (todos.length === 0) { return "📋 No tasks."; }
  var statusIcon = { "pending": "⬜", "in_progress": "🔄", "blocked": "🚫", "done": "✅" };
  var lines = ["📋 Task List:"];
  var done = 0;
  for (var i = 0; i < todos.length; i++) {
    var t = todos[i];
    lines.push("  " + (statusIcon[t.status] || "⬜") + " #" + t.id + " " + t.text);
    if (t.status === "done") { done++; }
  }
  lines.push("");
  lines.push("Progress: " + done + "/" + todos.length + " tasks complete");
  return lines.join("\n");
}
