//! 内嵌 HTML 静态资源

pub const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>EchoKit 设备配置</title>
    <style>
        * { box-sizing: border-box; margin: 0; padding: 0; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
            background: #1a1a2e;
            color: #eee;
            min-height: 100vh;
            padding: 20px;
        }
        .container {
            max-width: 400px;
            margin: 0 auto;
        }
        h1 {
            text-align: center;
            margin-bottom: 24px;
            font-size: 24px;
            color: #00d4ff;
        }
        .form-group {
            margin-bottom: 16px;
        }
        label {
            display: block;
            margin-bottom: 6px;
            font-size: 14px;
            color: #aaa;
        }
        input, select {
            width: 100%;
            padding: 12px;
            border: 1px solid #333;
            border-radius: 8px;
            background: #16213e;
            color: #fff;
            font-size: 16px;
        }
        input:focus, select:focus {
            outline: none;
            border-color: #00d4ff;
        }
        .button-group {
            display: flex;
            gap: 12px;
            margin-top: 24px;
        }
        button {
            flex: 1;
            padding: 14px;
            border: none;
            border-radius: 8px;
            font-size: 16px;
            cursor: pointer;
            min-height: 48px;
        }
        .btn-scan {
            background: #0f3460;
            color: #00d4ff;
        }
        .btn-save {
            background: #00d4ff;
            color: #1a1a2e;
            font-weight: bold;
        }
        button:disabled {
            opacity: 0.5;
            cursor: not-allowed;
        }
        #message {
            margin-top: 20px;
            padding: 12px;
            border-radius: 8px;
            text-align: center;
            display: none;
        }
        .success { background: #1e4d2b; color: #4ade80; display: block; }
        .error { background: #4d1e1e; color: #f87171; display: block; }
        .info { background: #1e3a5f; color: #60a5fa; display: block; }
        #device-info {
            background: #16213e;
            padding: 12px;
            border-radius: 8px;
            margin-bottom: 20px;
            font-size: 14px;
        }
        #device-info span { color: #00d4ff; }
        .loading { opacity: 0.7; pointer-events: none; }
    </style>
</head>
<body>
    <div class="container">
        <h1>EchoKit 设备配置</h1>

        <div id="device-info">
            版本: <span id="version">-</span>
        </div>

        <form id="config-form">
            <div class="form-group">
                <label>WiFi 网络</label>
                <input type="text" id="ssid" placeholder="输入 WiFi 名称" required>
            </div>

            <div class="form-group">
                <label>WiFi 密码</label>
                <input type="password" id="pass" placeholder="输入 WiFi 密码">
            </div>

            <div class="form-group">
                <label>服务器地址</label>
                <input type="text" id="server_url" placeholder="wss://your-server.com">
            </div>

            <div class="button-group">
                <button type="submit" class="btn-save" id="save-btn">保存配置</button>
            </div>
        </form>

        <div id="message"></div>
    </div>

    <script>
        const form = document.getElementById('config-form');
        const message = document.getElementById('message');
        const saveBtn = document.getElementById('save-btn');

        // 加载设备状态
        async function loadStatus() {
            try {
                const resp = await fetch('/api/status');
                const data = await resp.json();
                document.getElementById('version').textContent = data.version || '-';
                if (data.ssid) document.getElementById('ssid').value = data.ssid;
                if (data.server_url) document.getElementById('server_url').value = data.server_url;
            } catch (e) {
                console.error('Failed to load status:', e);
            }
        }

        // 显示消息
        function showMessage(text, type) {
            message.textContent = text;
            message.className = type;
        }

        // 提交配置
        form.addEventListener('submit', async (e) => {
            e.preventDefault();
            saveBtn.disabled = true;
            saveBtn.textContent = '保存中...';
            showMessage('正在保存配置...', 'info');

            try {
                const config = {
                    ssid: document.getElementById('ssid').value,
                    pass: document.getElementById('pass').value,
                    server_url: document.getElementById('server_url').value
                };

                const resp = await fetch('/api/config', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify(config)
                });

                if (resp.ok) {
                    showMessage('配置已保存，设备即将重启...', 'success');
                } else {
                    throw new Error('保存失败');
                }
            } catch (e) {
                showMessage('保存失败: ' + e.message, 'error');
                saveBtn.disabled = false;
                saveBtn.textContent = '保存配置';
            }
        });

        // 页面加载时获取状态
        loadStatus();
    </script>
</body>
</html>"#;
