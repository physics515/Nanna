# Stop Button Implementation Plan

## Backend Changes (gui/src-tauri/src/lib.rs)

1. Add to AppState struct:
```rust
pub struct AppState {
    // ... existing fields ...
    message_cancel_token: Arc<tokio::sync::RwLock<Option<tokio::sync::CancellationToken>>>,
}
```

2. Add new Tauri command:
```rust
#[tauri::command]
async fn cancel_message(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let app_state = state.read().await;
    if let Some(token) = app_state.message_cancel_token.read().await.as_ref() {
        token.cancel();
    }
    Ok(())
}
```

3. Modify send_message function:
   - Create new CancellationToken at start
   - Store it in app_state.message_cancel_token
   - Pass token to streaming logic
   - Check token.is_cancelled() in loop

## Frontend Changes (gui/app/pages/index.vue)

1. Add reactive state:
```typescript
const isStreaming = ref(false)
```

2. Add Stop button in template (in message input area):
```vue
<button 
  v-if="isStreaming"
  @click="stopMessage"
  class="px-3 py-2 bg-red-500 text-white rounded hover:bg-red-600"
>
  Stop
</button>
```

3. Add methods:
```typescript
async function stopMessage() {
  await invoke('cancel_message')
}
```

4. Update send_message call:
   - Set isStreaming = true before sending
   - Set isStreaming = false after response completes or errors

## Files to Modify
- gui/src-tauri/src/lib.rs (AppState, send_message, new cancel_message command)
- gui/app/pages/index.vue (UI, state, methods)
