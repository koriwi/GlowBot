Vendored from huawei-dongle-api 0.2.1:
https://github.com/Narf-AI/huawei-lte-api

GlowBot compatibility patches:

- Add typed `SmsSendRequest` and `SmsApi::send` for `/api/sms/send-sms`.
- Use Huawei's password-type-4 token-salted SHA-256 login encoding.
- Prefer `/api/webserver/SesTokInfo` so B311 sessions use the expected `TokInfo` token.
- Serialize login/logout with the required `<request>` root and accept both `username` and `Username` in login state.
- Add explicit elided lifetimes required by newer Rust lints.

The upstream crate remains licensed under MIT OR Apache-2.0.
