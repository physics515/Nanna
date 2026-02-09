# Stop Button Implementation for Nanna GUI

## Backend Changes (gui/src-tauri/src/lib.rs)

### 1. Update imports (around line 39)
Change:
```rust
use tokio::sync::RwLock;
```

To:
```rust
use tokio::sync::{CancellationToken, RwLock};
```

### 2. Add field to AppState struct (around line 888-920)
Add this line to the struct:
```rust
    cancellation_token: Arc<RwLock<Option<CancellationToken>>>,
```

### 3. Initialize in AppState creation
When creating AppState, add:
```rust
    cancellation_token: Arc::new(RwLock::new(None)),
```

### 4. Add cancel_message command (add before invoke_handler)
```rust
/// Cancel the current streaming message
#[tauri::command]
async fn cancel_message(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let app_state = state.read().await;
    let mut token = app_state.cancellation_token.write().await;
    if let Some(ct) = token.take() {
        ct.cancel();
    }
    Ok(())
}
```

### 5. Add to invoke_handler! macro (around line 7906)
Add `cancel_message,` to the list

### 6. Modify send_message to check cancellation
In the streaming loop, add a check:
```rust
// Check if cancelled
if let Some(ct) = &*app_state.cancellation_token.read().await {
    if ct.is_cancelled() {
        break;
    }
}
```

## Frontend Changes (gui/app/pages/index.vue)

### 1. Add to script section
```typescript
const isStreaming = ref(false)

// Add this function
async function stopMessage() {
    try {
        await invoke('cancel_message')
        isStreaming.value = false
    } catch (error) {
        console.error('Failed to cancel message:', error)
    }
}
```

### 2. Update send_message call
Wrap with streaming state:
```typescript
isStreaming.value = true
try {
    // ... existing send_message logic
} finally {
    isStreaming.value = false
}
```

### 3. Add Stop button to template
Add near the send button:
```vue
<button
    v-if="isStreaming"
    @click="stopMessage"
    class="px-4 py-2 bg-nanna-error/20 text-nanna-error hover:bg-nanna-error/30 rounded-lg transition-colors"
    title="Stop message generation"
>
    Stop
</button>
```

## Summary
This implementation adds:
- CancellationToken tracking in AppState
- cancel_message Tauri command to trigger cancellation
- Streaming loop checks for cancellation
- Vue UI button that calls cancel_message
- Proper state management for streaming status
