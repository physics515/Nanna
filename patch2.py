import sys
sys.stdout.reconfigure(encoding='utf-8')

# Patch agent_service.rs
f = 'crates/nanna-daemon/src/agent_service.rs'
content = open(f, encoding='utf-8').read()

# 1. chat_in_workspace: add attachments param
old1 = '''    pub async fn chat_in_workspace(
        &self,
        session_id: &str,
        message: &str,
        system_prompt: Option<String>,
        history: &[SessionMessage],
        workspace_id: Option<String>,
    ) -> Result<ChatResult, String> {
        self.chat_with_options(session_id, message, system_prompt, history, None, None, workspace_id).await
    }'''
new1 = '''    pub async fn chat_in_workspace(
        &self,
        session_id: &str,
        message: &str,
        system_prompt: Option<String>,
        history: &[SessionMessage],
        workspace_id: Option<String>,
        attachments: Vec<(String, String)>,
    ) -> Result<ChatResult, String> {
        self.chat_with_options(session_id, message, system_prompt, history, None, None, workspace_id, attachments).await
    }'''
content = content.replace(old1, new1)

# 2. chat_with_options: add attachments param
old2 = '''    pub async fn chat_with_options(
        &self,
        session_id: &str,
        message: &str,
        system_prompt: Option<String>,
        history: &[SessionMessage],
        model_override: Option<String>,
        max_iterations_override: Option<usize>,
        workspace_id: Option<String>,
    ) -> Result<ChatResult, String> {'''
new2 = '''    pub async fn chat_with_options(
        &self,
        session_id: &str,
        message: &str,
        system_prompt: Option<String>,
        history: &[SessionMessage],
        model_override: Option<String>,
        max_iterations_override: Option<usize>,
        workspace_id: Option<String>,
        attachments: Vec<(String, String)>,
    ) -> Result<ChatResult, String> {'''
content = content.replace(old2, new2)

# 3. Add attachments to RunOptions construction (before ..Default::default())
old3 = '''                max_iterations: max_iterations_override,
                ..Default::default()'''
new3 = '''                max_iterations: max_iterations_override,
                attachments,
                ..Default::default()'''
content = content.replace(old3, new3)

# 4. Fix any other calls to chat_with_options that don't have attachments yet
# The chat() method calls chat_with_options without the new param
old4 = 'self.chat_with_options(session_id, message, system_prompt, history, None, None, None).await'
new4 = 'self.chat_with_options(session_id, message, system_prompt, history, None, None, None, vec![]).await'
content = content.replace(old4, new4)

open(f, 'w', encoding='utf-8').write(content)
print('agent_service.rs updated')
