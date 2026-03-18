import sys
sys.stdout.reconfigure(encoding='utf-8')

f = 'crates/nanna-agent/src/loop_runner.rs'
lines = open(f, encoding='utf-8').readlines()

# 1. Add ImageSource to import (line 5, 0-indexed=4)
lines[4] = lines[4].replace('ContentBlock, LlmClient', 'ContentBlock, ImageSource, LlmClient')

# 2. Add attachments field to RunOptions (after cancellation_flag line)
for i, line in enumerate(lines):
    if 'pub cancellation_flag: Option<Arc<std::sync::atomic::AtomicBool>>,' in line:
        lines.insert(i + 1, '    /// Image attachments for the current message: Vec<(base64_data, media_type)>\n')
        lines.insert(i + 2, '    pub attachments: Vec<(String, String)>,\n')
        break

# 3. Replace user_text with conditional image support
for i, line in enumerate(lines):
    if 'ctx.messages.push(AnthropicMessage::user_text(msg));' in line:
        indent = '        '
        replacement = (
            indent + 'if options.attachments.is_empty() {\n' +
            indent + '    ctx.messages.push(AnthropicMessage::user_text(msg));\n' +
            indent + '} else {\n' +
            indent + '    // Build content blocks: text first, then images\n' +
            indent + '    let mut blocks = vec![ContentBlock::Text { text: msg }];\n' +
            indent + '    for (data, media_type) in &options.attachments {\n' +
            indent + '        blocks.push(ContentBlock::Image {\n' +
            indent + '            source: ImageSource::Base64 {\n' +
            indent + '                media_type: media_type.clone(),\n' +
            indent + '                data: data.clone(),\n' +
            indent + '            },\n' +
            indent + '        });\n' +
            indent + '    }\n' +
            indent + '    ctx.messages.push(AnthropicMessage::user(blocks));\n' +
            indent + '}\n'
        )
        lines[i] = replacement
        break

open(f, 'w', encoding='utf-8').writelines(lines)
print('loop_runner.rs updated')
