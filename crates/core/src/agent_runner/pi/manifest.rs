//! The files of the reviewed Pi packages, pinned by BLAKE3 hash (0.3, owner
//! decision 2026-11-18, pending item 9). Any difference stops classification
//! before Pi is started or sent any text.
//!
//! Pinned for 1.5.2: `package.json` and every file under `dist/bundle/`
//! (85 files, 9.4 MB): the `pi` bin (`dist/bundle/cli.js`), its runtime and
//! every chunk it can import. The whole package is 153 MB in about 15,500
//! files, mostly `node_modules/`; the bundle loads from there only optional
//! modules (image resizing, Negotiate proxy authentication, native WebSocket
//! helpers), none on the text-only path, and Kinana sets the two `WS_NO_*`
//! variables so the WebSocket helpers are not loaded. `node_modules/` is not
//! pinned (docs/pi-classification-proposal.md, "Known limits").
//!
//! Source: the installed package, identical file for file to the npm
//! tarball `@earendil-works/pi-coding-agent@1.5.2` (integrity
//! sha512-mUWI04rqF1Vd/m1q+Ps1XeWsTXEVvRUnsmRa83I3Dzua0Tx5bYS93imeqtIfrXf7PkkaR6ERz+AuQhIJlom5yA==),
//! checked on 2026-11-18.

pub(super) const VERSION_1_5_2: &str = "1.5.2";
pub(super) const BIN: &str = "dist/bundle/cli.js";
pub(super) const TREE: &str = "dist/bundle";

pub(super) const FILES_1_5_2: &[(&str, &str)] = &[
    (
        "dist/bundle/chunks/anthropic-messages-CA3HHOL3.js",
        "4c446f62c8209cbd3cba76946b5daf096dbb3fcac3a4b44f6609cc3822a7c8a1",
    ),
    (
        "dist/bundle/chunks/anthropic.js",
        "d4ab34f8978397b9d2624f00619965624bcc9ec3fa4c78c8e705deb2e1e3c90b",
    ),
    (
        "dist/bundle/chunks/azure-openai-responses-76TWS6DT.js",
        "a994c690c14c3f8347e5b836bcdbe58a0054d69cd5fd7c490ef98576b00cbf45",
    ),
    (
        "dist/bundle/chunks/bedrock-converse-stream.js",
        "ec64ea625b9ce0c9949bf110f7247ea39e3fce82a3443272526c1e09a281eacf",
    ),
    (
        "dist/bundle/chunks/chunk-36QC74V4.js",
        "68532dd2f010b3433ccd778cef27f1b196840a27f85931ed5a954d772aa738d9",
    ),
    (
        "dist/bundle/chunks/chunk-4KSTFUIK.js",
        "d3b88a1f08361a0ac073e711234c43ddafecdb1edc10623981dfd75b6ba10caa",
    ),
    (
        "dist/bundle/chunks/chunk-4UNPP53I.js",
        "0a6936e2464b1e4be32c67ed80e80ebc6a24ce6c98c90e898df325e59556aea9",
    ),
    (
        "dist/bundle/chunks/chunk-5AGIVGSV.js",
        "f840aed1d03226f77a333cd66d5dae1e5ee0c2b4d01d8ed1ac34fdcdf9a1d4bf",
    ),
    (
        "dist/bundle/chunks/chunk-6QGV3IWD.js",
        "8d728b7211853f7a29cf09a1e599dcca264e4d39b6f48b4a3e77cf32023f1f8d",
    ),
    (
        "dist/bundle/chunks/chunk-7CBJINYJ.js",
        "5401236e4da2cce35c20239d79bc636775de7bbaf6e134be36d94fd37ba24bbf",
    ),
    (
        "dist/bundle/chunks/chunk-7DDKXS2Q.js",
        "aa90f9e16cacad7839e197161694affdf6b2de411f7dc0950f645d6b1a1fbb42",
    ),
    (
        "dist/bundle/chunks/chunk-7ZC4UXSL.js",
        "966f65719e661df0e5eacd16731556941fd44c174f6132ec9b93b0a53e2d074e",
    ),
    (
        "dist/bundle/chunks/chunk-AUUCDC7R.js",
        "d91c1feea4642d14987be535db1d9d01588809884ec2ae9050c86a624aa97867",
    ),
    (
        "dist/bundle/chunks/chunk-BAC3QPNW.js",
        "cece5085f2d3c7e6b46d64c5b207f4bf6804e6bc854fa29535695844e5850b20",
    ),
    (
        "dist/bundle/chunks/chunk-CF5GWZHW.js",
        "f0d9f6e537df14738607fda124237a841191b1f716162e7aa12df548a6860453",
    ),
    (
        "dist/bundle/chunks/chunk-FF6XOTWB.js",
        "7733ce9622935ca294094eea24b3ac19a6f51cae71813e167d95de3d37f57654",
    ),
    (
        "dist/bundle/chunks/chunk-FFMDRXC4.js",
        "21a74b842af9d17846fdfa8be71a28b47b1c151de75c0e025d584601f2fe2000",
    ),
    (
        "dist/bundle/chunks/chunk-FMPAMX2W.js",
        "26191e864955d14fef77f298e365779437a56a05f19912c6c7dd683c9adeac27",
    ),
    (
        "dist/bundle/chunks/chunk-GFUN2N5K.js",
        "8afadb399c0085d9142244171ae17f086e4fe095897d87ee970f3030b44c9c21",
    ),
    (
        "dist/bundle/chunks/chunk-IEBK4HQA.js",
        "3b07961b26eb46ace687019f3ae47f6a56766abef9184c5109cea52e1db4209c",
    ),
    (
        "dist/bundle/chunks/chunk-IQUU5XIT.js",
        "6deb638f4c99876987b2c96cf421b3ec2062f88761dc6d162095e251fade88b3",
    ),
    (
        "dist/bundle/chunks/chunk-JXWJOIH4.js",
        "4bd7d1229982199bf257b9245f3ab6f3f504e60d7d952da1dec2622c86e42093",
    ),
    (
        "dist/bundle/chunks/chunk-L2ZPNLYD.js",
        "1844f76b5205621f5436945573f6f6ccda8593253c8460a026cb232c4b1c4830",
    ),
    (
        "dist/bundle/chunks/chunk-LPXNMA2W.js",
        "7d7e51bf2069d67fa4cae020fb7c5aa4547b6474637b2b69bc845f14f296cb50",
    ),
    (
        "dist/bundle/chunks/chunk-LQLJ7CDW.js",
        "a313d66c949de96122ae341cf0550c74cf688718d46a3a5f33222c4e1bea54c1",
    ),
    (
        "dist/bundle/chunks/chunk-LVVZS2NO.js",
        "57217486b8062a238c18ca66a4465e117da44077427ed1a4aafbc0ab967c7fa8",
    ),
    (
        "dist/bundle/chunks/chunk-MMDXLZVS.js",
        "d364147c9270cd5aa73d78c41801fce03054711c81854beb0f829afb16bf2179",
    ),
    (
        "dist/bundle/chunks/chunk-P7FPUEYA.js",
        "b5177b84899ce4c1028fcb2af6b8916400f8cbca3e367cd5e74f7cd9520b545f",
    ),
    (
        "dist/bundle/chunks/chunk-PGXDEOVV.js",
        "f0b47433d78fb06285591c50d978d76c68dc792132583a57299a9c76e90aec3c",
    ),
    (
        "dist/bundle/chunks/chunk-PJACZCIR.js",
        "d2343e89fc8314710e8472fa9e6bacd08f0143c61fc78b1bf8731d46870e2923",
    ),
    (
        "dist/bundle/chunks/chunk-PMRZO46Y.js",
        "5729636f1076a79fbda0f750c875d7ddd3396244177c749e1db9752d13617511",
    ),
    (
        "dist/bundle/chunks/chunk-PRRNXB7S.js",
        "13b9610ca269aec6b6438b5dbebd6b662a80e2fa892fe14163b745516bfb7478",
    ),
    (
        "dist/bundle/chunks/chunk-QN4TOIZN.js",
        "cc1e32638f1cc735bb61ca87ebd6dde3b7da5fc4316eb9e84038881fa10fc39d",
    ),
    (
        "dist/bundle/chunks/chunk-RDZVAGPR.js",
        "4ca33f0d3352601de80f7209048f923d3d12324f0581b44f9762f09a8c5a0bd1",
    ),
    (
        "dist/bundle/chunks/chunk-RKNK4Y6L.js",
        "77ce122a5932e99f3f3b62d90f4e73fb94ed561742cc676886c08dd3f27206f9",
    ),
    (
        "dist/bundle/chunks/chunk-TMTOQ7H7.js",
        "916767615577e60505f1c35ce0450f6ddd66e694c06439d9d2fab6e3b838bbf8",
    ),
    (
        "dist/bundle/chunks/chunk-UTB4DTYQ.js",
        "e92fc910b38df67fe5620f4d1ba03de1d3e83c18fcb31c034209de13c77b5263",
    ),
    (
        "dist/bundle/chunks/chunk-V7SLUK6A.js",
        "b537786696a153e3dcbb87149d4634e4470cb173ec48a3c35ea03b8c5b9dacb0",
    ),
    (
        "dist/bundle/chunks/chunk-VEJDBY3O.js",
        "893bbb37d9dd5d85eb986c057eca376e85f9655c0f7a6f0dddba1192bca21202",
    ),
    (
        "dist/bundle/chunks/chunk-VKJQ6CME.js",
        "22a54b6012d6d9da6156156a5735dd725f75c8bf83c3b8c4cd5ddd868d317f1c",
    ),
    (
        "dist/bundle/chunks/chunk-WL4TUXY7.js",
        "d11a0e6550a76925898403119e8894facf67feac492adca75a25b04c8f193b57",
    ),
    (
        "dist/bundle/chunks/chunk-Y55SJART.js",
        "db22cc16904ad2ec54496e9959425d41d42d44aeba1a418722c536e1bcd43d44",
    ),
    (
        "dist/bundle/chunks/chunk-ZBIZYD6W.js",
        "7948d9e5fa4d926683267c6e75eccc258be523f0e0f6f25be9f171cd8d706b8c",
    ),
    (
        "dist/bundle/chunks/chunk-ZG74NES2.js",
        "e67021d96211abc180c96169b14cfa63d8ace47176f9421c3c20b69c16b6f1f6",
    ),
    (
        "dist/bundle/chunks/chunk-ZGYS4UTC.js",
        "12666ec4b473d8f1c43ee5ad9f3a0b379b142e1fabf0f74cfe757604c457593e",
    ),
    (
        "dist/bundle/chunks/cli-7RHMDOHL.js",
        "281ff0fe3978cd6daba14186953bfe93da985900050f9b07b46584cd359a66db",
    ),
    (
        "dist/bundle/chunks/cloudflare-workers-ai-system-one-U4J7D646.js",
        "324333027dda5fa8ee193a14df31b8b373a1215cf474933aef09857cc2ee25e4",
    ),
    (
        "dist/bundle/chunks/codemode-worker.js",
        "3de959db819c6d6c62adbb193caf0d0b31d882cef2ea8d2a9c1292468e0efd8d",
    ),
    (
        "dist/bundle/chunks/easter-egg-3d-GEVPKQF6.js",
        "f892715fcc9f662de762f4596e48fcb69f51bdc3174e2511ffc34ddea7e35f28",
    ),
    (
        "dist/bundle/chunks/execute-L2EEDV3L.js",
        "5eccee82a5160ac8dc3baed7237e4d41c5a663084847cfe77f95529b951962a4",
    ),
    (
        "dist/bundle/chunks/github-copilot.js",
        "238a64e82b74ad800ac55c4b4509d1b79ef307390454c2e8d0ac89a1074975a0",
    ),
    (
        "dist/bundle/chunks/google-generative-ai-D5B6W47J.js",
        "add9b713bc0ccc40621c10b3e28a5ed11e6d76ebeebac65f3c8f5344cba57183",
    ),
    (
        "dist/bundle/chunks/google-vertex-U4H5YDEW.js",
        "9621954dcd1d3a7802893f40513ff7e4fe8a5314aa3bcfdc9e3f7b00f8a85f9c",
    ),
    (
        "dist/bundle/chunks/https-proxy-agent-LVOO5TPL.js",
        "a4b8949cd94aced3ee88f1b2cef0168c952714634bd7bc054641bead493e414b",
    ),
    (
        "dist/bundle/chunks/image-resize-worker.js",
        "e4c8f06ed99aa9bca2bed0e4194a24e75bd67ea7b8c099e8e0980f0059180825",
    ),
    (
        "dist/bundle/chunks/jiti-loader-GL4HJFAD.js",
        "21117121e0fb6369b3868f6cc30eeb2a2c3243d837364e5a3e00725e943a3a8f",
    ),
    (
        "dist/bundle/chunks/jiti-static-loader-2RCDI6B4.js",
        "5ebd50d305b678eca37e6f150663350ee75ff0f98d6450d801d7bafb0ce944d5",
    ),
    (
        "dist/bundle/chunks/kimi-coding.js",
        "dea47a24e3ec31a87a67e25cbad29b122ad813c4c1a08d87045044cfb103e574",
    ),
    (
        "dist/bundle/chunks/lib-4AVRLBHM.js",
        "5877648e61b477c2333cd4d75d74c4cb489d50ea7ea71cc630416aea2ea224c4",
    ),
    (
        "dist/bundle/chunks/llama-cpp-classify-EJXCQRUQ.js",
        "78be63fca6c2e8866a4cb8faf516be3dbba56e181543ab00a2b373f623e73c4e",
    ),
    (
        "dist/bundle/chunks/meta.js",
        "73edcabffbf60066f03c9331f449c09687cc011c76b720a2b51aee8e53716e8b",
    ),
    (
        "dist/bundle/chunks/mistral-conversations-S277EKLB.js",
        "ea010028f219f0b302b1c92586fba37f9bc117f48abe96648084bad3c09b2951",
    ),
    (
        "dist/bundle/chunks/multipart-parser-FMGQ7342.js",
        "1bffb7192100719d935f0af9013bf14f9396ff3df839566f06c2b7f2b59d24f2",
    ),
    (
        "dist/bundle/chunks/node-AUOGOBCN.js",
        "28cd46f0516a8bf24210c98a5d48525cbe497ce35f16f89ab3b9b2ef360627f3",
    ),
    (
        "dist/bundle/chunks/node-KCMANYC2.js",
        "790b2f22d360ab7dfbb76d9851929c1c9bb53a50731978542de96c2536e77ded",
    ),
    (
        "dist/bundle/chunks/openai-chatgpt.js",
        "42d8e677b0bc3906c1753b98d3d4e7e31b736b3dafc0c2e4ec537377eff57957",
    ),
    (
        "dist/bundle/chunks/openai-codex-responses-TEQRB65N.js",
        "12cf24b4d439c03f6148c86733805879d60329013a4e871a326572aac1638945",
    ),
    (
        "dist/bundle/chunks/openai-codex.js",
        "db56194893acb3ea9a6ddd826ae92d589faa9ed5b2d3ef427e61560e43dc7339",
    ),
    (
        "dist/bundle/chunks/openai-completions-HRZOSLBO.js",
        "25739f2ba6f9fd173ab12658118f0d355ba2e3af411dd6a204cc729f5a098994",
    ),
    (
        "dist/bundle/chunks/openai-decisions-F66BDLOM.js",
        "cc9ede158b455bd1098458e702961ef17903a27067446b97a48df1e590d3d37c",
    ),
    (
        "dist/bundle/chunks/openai-responses-BAKFWTGE.js",
        "ff33e5927e88df5d057b02836ec2aee77a9b21ab3ca2b45b21f682e7cf7493fc",
    ),
    (
        "dist/bundle/chunks/openrouter-images-VXF5TVIK.js",
        "81247d40708f70268e81cdf6f7ada7260dbe950e0db4361a041cc650a3d5779a",
    ),
    (
        "dist/bundle/chunks/openrouter.js",
        "c4ba5419658ae6b0f3071f07b119221630c7e7e17fa54ec70d5df197362b8958",
    ),
    (
        "dist/bundle/chunks/pi-messages-UBJKNDRN.js",
        "7b24a5610501a91a733f0d99c36311778facacb2dca6ee7e783cbe0ac625c1ff",
    ),
    (
        "dist/bundle/chunks/radius.js",
        "159a5aab370771b84a2292b1ea21532f515c978aab440c22654d843d28a1dfbf",
    ),
    (
        "dist/bundle/chunks/runtime-J5ZSBGTX.js",
        "f618bfae7cea7b4078db6920d7f1763831e6ad3f1b4bca595fe9300b8ab8880b",
    ),
    (
        "dist/bundle/chunks/src-QKGKDFSU.js",
        "b06dc2cc6054d276cbc407a8b15e882f1783de4ccc4392b20e8cee3e2309a43b",
    ),
    (
        "dist/bundle/chunks/typesafe-system-one-56TEMIHL.js",
        "fb68b30ba1ced47c862dcef11d95047b05a627f0a24de2387bdf782a3e3791bd",
    ),
    (
        "dist/bundle/chunks/virtual-modules-KITQRTYM.js",
        "af1853d683d347aed35530aa85ebed37188043d8484859cd5ab2c2273af8f262",
    ),
    (
        "dist/bundle/chunks/xai.js",
        "ac763030c179cb3ac8a70416f89e1a5d3fbf6c5c3b8e6ccbd6a5bc6a44d0b78c",
    ),
    (
        "dist/bundle/cli-runtime.js",
        "5b4ab15567ddab5ae2ee059b9c81538ed0af75afc543d7890887b8a12ae040bf",
    ),
    (
        "dist/bundle/cli.js",
        "2f498a2ed8783ac8009b3c0c5dfabd5aec0de01f3e98b055363f8d44ea7720f6",
    ),
    (
        "dist/bundle/index.js",
        "7f9c2ce08408c3cc0fe54e439b7a50e3c37fc99cab42c19e0c089bb034720031",
    ),
    (
        "dist/bundle/rpc-entry.js",
        "228933c9b0f6a2e526940997a04e83bc047006878b7483be13d58c347356e88b",
    ),
    (
        "package.json",
        "16f53e7b9d1ec8d05f17ee5493b4b921c236ae2947f32f15d38bc6ed2f0aed30",
    ),
];

/// Pi 1.5.2's built-in default chat model per provider (owner, 2026-11-24:
/// the model is optional and "Pi default" is offered). Pi 1.5.2 refuses
/// `--provider` without `--model` (main.js `buildSessionOptions`), so Kinana
/// passes this model explicitly. Copied from `defaultModelPerProvider` in
/// `dist/core/model-resolver.js`; the same table is in the pinned
/// `dist/bundle/chunks/chunk-VEJDBY3O.js`. Sorted by provider.
pub(super) const DEFAULT_MODELS_1_5_2: &[(&str, &str)] = &[
    ("amazon-bedrock", "us.anthropic.claude-opus-5-5"),
    ("ant-ling", "Ring-2.6-1T"),
    ("anthropic", "claude-opus-5-5"),
    ("azure", "gpt-6.1-sol"),
    ("baseten", "zai-org/GLM-5.3"),
    ("cerebras", "gpt-oss-120b"),
    (
        "cloudflare-ai-gateway",
        "workers-ai/@cf/moonshotai/kimi-k2.7-code",
    ),
    ("cloudflare-workers-ai", "@cf/moonshotai/kimi-k2.7-code"),
    ("deepseek", "deepseek-v4-pro"),
    ("fireworks", "accounts/fireworks/models/kimi-k3"),
    ("github-copilot", "gpt-6.1-sol"),
    ("google", "gemini-4-argon"),
    ("google-vertex", "gemini-4-argon"),
    ("groq", "openai/gpt-oss-120b"),
    ("huggingface", "moonshotai/Kimi-K2.7-Code"),
    ("kimi-coding", "kimi-for-coding"),
    ("meta", "muse-spark-1.3"),
    ("minimax", "MiniMax-M3"),
    ("minimax-cn", "MiniMax-M3"),
    ("mistral", "devstral-medium-latest"),
    ("moonshotai", "kimi-k2.7-code"),
    ("moonshotai-cn", "kimi-k2.7-code"),
    ("nvidia", "nvidia/nemotron-3.5-super-vl-120b-a12b"),
    ("openai", "gpt-6.1-sol"),
    ("openai-codex", "gpt-6.1-sol"),
    ("opencode", "kimi-k2.7-code"),
    ("opencode-go", "kimi-k3.1-pro"),
    ("openrouter", "moonshotai/kimi-k2.7-code"),
    ("qwen-token-plan", "qwen3.8-max"),
    ("qwen-token-plan-cn", "qwen3.8-max"),
    ("qwen-token-plan-individual", "qwen3.8-max"),
    ("qwen-token-plan-individual-cn", "qwen3.8-max"),
    ("radius", "balanced"),
    ("together", "moonshotai/Kimi-K3"),
    ("vercel-ai-gateway", "zai/glm-5.3"),
    ("xai", "grok-4.7"),
    ("xiaomi", "mimo-v2.6-pro"),
    ("xiaomi-token-plan-ams", "mimo-v2.6-pro"),
    ("xiaomi-token-plan-cn", "mimo-v2.6-pro"),
    ("xiaomi-token-plan-sgp", "mimo-v2.6-pro"),
    ("zai", "glm-5.3"),
    ("zai-coding-cn", "glm-5.3"),
];

pub(super) const VERSION_1_6_1: &str = "1.6.1";
// From the reviewed npm archive and evidence/0.4-pi-c15/pin-manifest-1.6.1.json.
pub(super) const FILES_1_6_1: &[(&str, &str)] = &[
    (
        "dist/bundle/chunks/anthropic-messages-22H2E23B.js",
        "27590dedb74c13ec719b18e3962169bb64137042228b09e29d21f869be006139",
    ),
    (
        "dist/bundle/chunks/anthropic.js",
        "d4ab34f8978397b9d2624f00619965624bcc9ec3fa4c78c8e705deb2e1e3c90b",
    ),
    (
        "dist/bundle/chunks/azure-openai-responses-JJ726GYJ.js",
        "11bbf117c9c68bbb0958ee245b09c752bd5f86814aac259a75f5969b6f641bdc",
    ),
    (
        "dist/bundle/chunks/bedrock-converse-stream.js",
        "ae6bf3e45e7d9a6386aa23f800785e9e09f8871ff2cdb6c2bb500428a8bc770e",
    ),
    (
        "dist/bundle/chunks/chunk-2BKJ4UF2.js",
        "47d6af6230557820428a6fbf460acdce11bcf795417f3303ef49b4dbf6de856d",
    ),
    (
        "dist/bundle/chunks/chunk-36QC74V4.js",
        "68532dd2f010b3433ccd778cef27f1b196840a27f85931ed5a954d772aa738d9",
    ),
    (
        "dist/bundle/chunks/chunk-52KD6VTT.js",
        "874154ed9258ab193f708394956b4a0f2dfdc126140b093d5402226737e8532c",
    ),
    (
        "dist/bundle/chunks/chunk-5AGIVGSV.js",
        "f840aed1d03226f77a333cd66d5dae1e5ee0c2b4d01d8ed1ac34fdcdf9a1d4bf",
    ),
    (
        "dist/bundle/chunks/chunk-5GJXA75I.js",
        "08b04a02014b555620be44f4be572f301c0b538b166831a8f1e696199e1017ce",
    ),
    (
        "dist/bundle/chunks/chunk-65LZPN67.js",
        "7b4150c197cfc17f9b93c81558bd2c3c14851719bb8e5318cdb69b7e0ae14a0d",
    ),
    (
        "dist/bundle/chunks/chunk-6BFP5YI5.js",
        "97d3c8a9beb3e44c3fd52335dc0c1c6c9a47fd152c019577dc33ad0abc3d82f5",
    ),
    (
        "dist/bundle/chunks/chunk-6QGV3IWD.js",
        "8d728b7211853f7a29cf09a1e599dcca264e4d39b6f48b4a3e77cf32023f1f8d",
    ),
    (
        "dist/bundle/chunks/chunk-7CBJINYJ.js",
        "5401236e4da2cce35c20239d79bc636775de7bbaf6e134be36d94fd37ba24bbf",
    ),
    (
        "dist/bundle/chunks/chunk-7DDKXS2Q.js",
        "aa90f9e16cacad7839e197161694affdf6b2de411f7dc0950f645d6b1a1fbb42",
    ),
    (
        "dist/bundle/chunks/chunk-7ZC4UXSL.js",
        "966f65719e661df0e5eacd16731556941fd44c174f6132ec9b93b0a53e2d074e",
    ),
    (
        "dist/bundle/chunks/chunk-DPMEYYP2.js",
        "1b3276078224518e2bb8abb737ce436a1bcc22060003fdab9450ae015c22d035",
    ),
    (
        "dist/bundle/chunks/chunk-EQ2BR3SF.js",
        "33428783839ce254e4a736fe258a2b436523ccd582821f51e1805677a57bc3bc",
    ),
    (
        "dist/bundle/chunks/chunk-ESCXDUHA.js",
        "f51ee1cb39f118c0be71799d6d4b9ec1ba39e23a9d37fcce9649837bb5a840d6",
    ),
    (
        "dist/bundle/chunks/chunk-FFMDRXC4.js",
        "21a74b842af9d17846fdfa8be71a28b47b1c151de75c0e025d584601f2fe2000",
    ),
    (
        "dist/bundle/chunks/chunk-GH2VPQ2P.js",
        "b411dded94e4cbde6d032ee1de242faa26499b59cf0243252da1e3f996e5f048",
    ),
    (
        "dist/bundle/chunks/chunk-HID5BUPP.js",
        "9f7ddbc055ec689d8ca7e35d4f9236ee6e458b6d236946daf6bafec95125f822",
    ),
    (
        "dist/bundle/chunks/chunk-IEBK4HQA.js",
        "3b07961b26eb46ace687019f3ae47f6a56766abef9184c5109cea52e1db4209c",
    ),
    (
        "dist/bundle/chunks/chunk-INODY4DR.js",
        "e3231973be6d08dbb8bce6442df56a8b66fc351fc4d6e7c53b72e3597df1c40a",
    ),
    (
        "dist/bundle/chunks/chunk-JXWJOIH4.js",
        "4bd7d1229982199bf257b9245f3ab6f3f504e60d7d952da1dec2622c86e42093",
    ),
    (
        "dist/bundle/chunks/chunk-L2ZPNLYD.js",
        "1844f76b5205621f5436945573f6f6ccda8593253c8460a026cb232c4b1c4830",
    ),
    (
        "dist/bundle/chunks/chunk-LPXNMA2W.js",
        "7d7e51bf2069d67fa4cae020fb7c5aa4547b6474637b2b69bc845f14f296cb50",
    ),
    (
        "dist/bundle/chunks/chunk-LVVZS2NO.js",
        "57217486b8062a238c18ca66a4465e117da44077427ed1a4aafbc0ab967c7fa8",
    ),
    (
        "dist/bundle/chunks/chunk-LWG2CUTI.js",
        "98c6970617823af29aa19e25bdbb7a83441b98db3c4223bd377972c4248cf824",
    ),
    (
        "dist/bundle/chunks/chunk-O5MXS5TI.js",
        "19825c3af3f6738ee6319fcc7427a6b4bd5ececae2de5b628af9dc9030da9963",
    ),
    (
        "dist/bundle/chunks/chunk-ORY5MLDF.js",
        "93ac74590684392332be391f9486e399666b98e0096051f359ba81a2ccbe6d9c",
    ),
    (
        "dist/bundle/chunks/chunk-OVXT2WQH.js",
        "3e354dd7492ba59eb85688312d9dfa6b7ce6faf5d46762b378fc23b32e6718fb",
    ),
    (
        "dist/bundle/chunks/chunk-P7FPUEYA.js",
        "b5177b84899ce4c1028fcb2af6b8916400f8cbca3e367cd5e74f7cd9520b545f",
    ),
    (
        "dist/bundle/chunks/chunk-PB5262RA.js",
        "603e6d6a3dfdfee8583d09e19f74cb28b82cc0a69b5bfbe30e65efe2f444f216",
    ),
    (
        "dist/bundle/chunks/chunk-PGXDEOVV.js",
        "f0b47433d78fb06285591c50d978d76c68dc792132583a57299a9c76e90aec3c",
    ),
    (
        "dist/bundle/chunks/chunk-PJACZCIR.js",
        "d2343e89fc8314710e8472fa9e6bacd08f0143c61fc78b1bf8731d46870e2923",
    ),
    (
        "dist/bundle/chunks/chunk-PRRNXB7S.js",
        "13b9610ca269aec6b6438b5dbebd6b662a80e2fa892fe14163b745516bfb7478",
    ),
    (
        "dist/bundle/chunks/chunk-RDZVAGPR.js",
        "4ca33f0d3352601de80f7209048f923d3d12324f0581b44f9762f09a8c5a0bd1",
    ),
    (
        "dist/bundle/chunks/chunk-TMTOQ7H7.js",
        "916767615577e60505f1c35ce0450f6ddd66e694c06439d9d2fab6e3b838bbf8",
    ),
    (
        "dist/bundle/chunks/chunk-UTB4DTYQ.js",
        "e92fc910b38df67fe5620f4d1ba03de1d3e83c18fcb31c034209de13c77b5263",
    ),
    (
        "dist/bundle/chunks/chunk-VARKPLD5.js",
        "3cbe0054cea30ba1ac09a8a51f7e9b756462aff5bea9874f8697f02d5d52b877",
    ),
    (
        "dist/bundle/chunks/chunk-VUDMZO5Q.js",
        "12d2fc923f4d58d56f8f56f6aab141b84dd5fb8196b85fa7d7e70f87fbf46aad",
    ),
    (
        "dist/bundle/chunks/chunk-VUZTGLGT.js",
        "497863ab61d06dea505c5f3bf276e88ce300a9be94004308c946326ee87bf48a",
    ),
    (
        "dist/bundle/chunks/chunk-WLFBMTZ5.js",
        "8d8a6f2728bb9dedbcf6090635baecbfb5b5baf5fa8ed4b052497f076fe86af0",
    ),
    (
        "dist/bundle/chunks/chunk-YXY4M7CD.js",
        "1ffc7271db321a65a24e56cac19a612aeb7a3037f0c9db6fde09b05ff2302ad1",
    ),
    (
        "dist/bundle/chunks/chunk-ZCDRM3TR.js",
        "136b09c6d30f3ab958655336629d9f57089733a88ac9ed8e0a14213907f45c47",
    ),
    (
        "dist/bundle/chunks/chunk-ZG74NES2.js",
        "e67021d96211abc180c96169b14cfa63d8ace47176f9421c3c20b69c16b6f1f6",
    ),
    (
        "dist/bundle/chunks/cli-3M6SYLOW.js",
        "e20678b412dd50e9cc56467443edabe6190a007efcb2b31db61bcc9cbe298153",
    ),
    (
        "dist/bundle/chunks/cloudflare-workers-ai-system-one-XWHRMCHY.js",
        "9313c7f74057bd96ca064e8481648f2554fbcdcb7aabffd67d41cc49cfa4310b",
    ),
    (
        "dist/bundle/chunks/codemode-worker.js",
        "3de959db819c6d6c62adbb193caf0d0b31d882cef2ea8d2a9c1292468e0efd8d",
    ),
    (
        "dist/bundle/chunks/easter-egg-3d-JQSUSCKG.js",
        "298d41f99d24eb5107db0762c6a7c4a3b8cda9a290d477523e436fdf707f0553",
    ),
    (
        "dist/bundle/chunks/execute-GM225T4M.js",
        "ada817e28b3013fbc69b2a422a5bf961fe68a7847efb794a61af21c262379597",
    ),
    (
        "dist/bundle/chunks/github-copilot.js",
        "6dbd994a7b8d02aebc2b330ea96524ff7a59738777d6446f29ec5beb03cc6b53",
    ),
    (
        "dist/bundle/chunks/google-generative-ai-YKXQ53ZT.js",
        "ec2108e65e2419fb95b5c737ee5c4d1dc41731cbfe7f2251fb63f9d43d52bd47",
    ),
    (
        "dist/bundle/chunks/google-vertex-MUCLHVXZ.js",
        "f9193da70ea4837d9764aa8abba9ce98a26b0d22cef1788e75b475bfc4e687d3",
    ),
    (
        "dist/bundle/chunks/https-proxy-agent-LVOO5TPL.js",
        "a4b8949cd94aced3ee88f1b2cef0168c952714634bd7bc054641bead493e414b",
    ),
    (
        "dist/bundle/chunks/image-resize-worker.js",
        "e4c8f06ed99aa9bca2bed0e4194a24e75bd67ea7b8c099e8e0980f0059180825",
    ),
    (
        "dist/bundle/chunks/jiti-loader-GL4HJFAD.js",
        "21117121e0fb6369b3868f6cc30eeb2a2c3243d837364e5a3e00725e943a3a8f",
    ),
    (
        "dist/bundle/chunks/jiti-static-loader-2RCDI6B4.js",
        "5ebd50d305b678eca37e6f150663350ee75ff0f98d6450d801d7bafb0ce944d5",
    ),
    (
        "dist/bundle/chunks/kimi-coding.js",
        "dea47a24e3ec31a87a67e25cbad29b122ad813c4c1a08d87045044cfb103e574",
    ),
    (
        "dist/bundle/chunks/lib-4AVRLBHM.js",
        "5877648e61b477c2333cd4d75d74c4cb489d50ea7ea71cc630416aea2ea224c4",
    ),
    (
        "dist/bundle/chunks/llama-cpp-classify-WIKNSO3N.js",
        "af704605dd3c8e8d3a59833494ebdbfec29877abc38cfe75b6cf772e98b42e9c",
    ),
    (
        "dist/bundle/chunks/meta.js",
        "73edcabffbf60066f03c9331f449c09687cc011c76b720a2b51aee8e53716e8b",
    ),
    (
        "dist/bundle/chunks/mistral-conversations-P4MOPI2E.js",
        "2965298307bb88584676bba148874cb21aefe10c9df5d38c2f054e0b8b5b4542",
    ),
    (
        "dist/bundle/chunks/multipart-parser-FMGQ7342.js",
        "1bffb7192100719d935f0af9013bf14f9396ff3df839566f06c2b7f2b59d24f2",
    ),
    (
        "dist/bundle/chunks/node-AUOGOBCN.js",
        "28cd46f0516a8bf24210c98a5d48525cbe497ce35f16f89ab3b9b2ef360627f3",
    ),
    (
        "dist/bundle/chunks/node-KCMANYC2.js",
        "790b2f22d360ab7dfbb76d9851929c1c9bb53a50731978542de96c2536e77ded",
    ),
    (
        "dist/bundle/chunks/openai-chatgpt.js",
        "42d8e677b0bc3906c1753b98d3d4e7e31b736b3dafc0c2e4ec537377eff57957",
    ),
    (
        "dist/bundle/chunks/openai-codex-responses-C5LXA4X5.js",
        "aa7939bf098206f8ac87872b6bbceada66cc538502f9258f4df18f4897f8a7cc",
    ),
    (
        "dist/bundle/chunks/openai-codex.js",
        "db56194893acb3ea9a6ddd826ae92d589faa9ed5b2d3ef427e61560e43dc7339",
    ),
    (
        "dist/bundle/chunks/openai-completions-DWXQWGRR.js",
        "e33719b8c0596558815fb8791fecfe9b6c4c0bc7772452326cbd348c47223752",
    ),
    (
        "dist/bundle/chunks/openai-decisions-OKDRL5TQ.js",
        "61d49dfb308fcdbf7cefc4dea3fb5cb0db3f88d188372978386ea76c5cb09529",
    ),
    (
        "dist/bundle/chunks/openai-responses-VJY3MQZW.js",
        "efae5f09a1fe274983444a77fc4636bd557c94e530fdfd86821a10e0f3b5cb32",
    ),
    (
        "dist/bundle/chunks/openrouter-images-7M2P66JO.js",
        "62ee69dba1980cb32b642cc1499a24c39fb339a96959863d63c0597d1ae00b4b",
    ),
    (
        "dist/bundle/chunks/openrouter.js",
        "c4ba5419658ae6b0f3071f07b119221630c7e7e17fa54ec70d5df197362b8958",
    ),
    (
        "dist/bundle/chunks/pi-decision-KNTK2XHG.js",
        "9ccf301973e3619c1236c4443df677fb902ee736889a29ff4fef9663d788fc85",
    ),
    (
        "dist/bundle/chunks/pi-messages-QJYPAHJF.js",
        "6081e9f38955499e114562571ae5697bd8535aa3b1970664ebbbe0c7651b12c6",
    ),
    (
        "dist/bundle/chunks/radius.js",
        "159a5aab370771b84a2292b1ea21532f515c978aab440c22654d843d28a1dfbf",
    ),
    (
        "dist/bundle/chunks/runtime-T6TR2GDO.js",
        "d34f5e43fe2fff049ee21e49be6cce2a1b0414b3566790efa185b8ed53edfaae",
    ),
    (
        "dist/bundle/chunks/src-QKGKDFSU.js",
        "b06dc2cc6054d276cbc407a8b15e882f1783de4ccc4392b20e8cee3e2309a43b",
    ),
    (
        "dist/bundle/chunks/typesafe-system-one-H25HBWGT.js",
        "12a7aff3ba6ee318d888d48624aefd754d5421d340662cb8734f142acdbfab5a",
    ),
    (
        "dist/bundle/chunks/virtual-modules-VWKYRS23.js",
        "d964a2c5eee822e5742fd8175521612b7a11158f6b540430c1111a3634ecfad6",
    ),
    (
        "dist/bundle/chunks/xai.js",
        "ac763030c179cb3ac8a70416f89e1a5d3fbf6c5c3b8e6ccbd6a5bc6a44d0b78c",
    ),
    (
        "dist/bundle/cli-runtime.js",
        "37dc915648095fefaaa9ba9ddc8d9dbd955439f12c8420a35ca77e22da567817",
    ),
    (
        "dist/bundle/cli.js",
        "2f498a2ed8783ac8009b3c0c5dfabd5aec0de01f3e98b055363f8d44ea7720f6",
    ),
    (
        "dist/bundle/index.js",
        "f07b2e972b2272849910b732ca86d56694b36bb2e76305d4fc9cab1a07dbe580",
    ),
    (
        "dist/bundle/rpc-entry.js",
        "ca17397569cea869c10138aec130a5e895a3c286acd498c8746f3945267c34ff",
    ),
    (
        "package.json",
        "6b960ab72477baa34582bb54f5f4355a2fb00992d1d21d53f1c4209b244d5c1b",
    ),
];
pub(super) const DEFAULT_MODELS_1_6_1: &[(&str, &str)] = &[
    ("amazon-bedrock", "us.anthropic.claude-opus-5-5"),
    ("ant-ling", "Ring-2.6-1T"),
    ("anthropic", "claude-opus-5-5"),
    ("azure", "gpt-6.2-sol"),
    ("baseten", "zai-org/GLM-5.3"),
    ("cerebras", "gpt-oss-120b"),
    (
        "cloudflare-ai-gateway",
        "workers-ai/@cf/moonshotai/kimi-k2.7-code",
    ),
    ("cloudflare-workers-ai", "@cf/moonshotai/kimi-k2.7-code"),
    ("deepseek", "deepseek-v4-pro"),
    ("fireworks", "accounts/fireworks/models/kimi-k3"),
    ("github-copilot", "gpt-6.1-sol"),
    ("google", "gemini-4-argon"),
    ("google-vertex", "gemini-4-argon"),
    ("groq", "openai/gpt-oss-120b"),
    ("huggingface", "moonshotai/Kimi-K2.7-Code"),
    ("kimi-coding", "kimi-for-coding"),
    ("meta", "muse-spark-1.3"),
    ("minimax", "MiniMax-M3"),
    ("minimax-cn", "MiniMax-M3"),
    ("mistral", "mistral-medium-latest"),
    ("moonshotai", "kimi-k2.7-code"),
    ("moonshotai-cn", "kimi-k2.7-code"),
    ("nvidia", "nvidia/nemotron-3.5-super-vl-120b-a12b"),
    ("openai", "gpt-6.2-sol"),
    ("openai-codex", "gpt-6.2-sol"),
    ("opencode", "kimi-k2.7-code"),
    ("opencode-go", "kimi-k3.1-pro"),
    ("openrouter", "moonshotai/kimi-k2.7-code"),
    ("qwen-token-plan", "qwen3.8-max"),
    ("qwen-token-plan-cn", "qwen3.8-max"),
    ("qwen-token-plan-individual", "qwen3.8-max"),
    ("qwen-token-plan-individual-cn", "qwen3.8-max"),
    ("radius", "balanced"),
    ("together", "moonshotai/Kimi-K3"),
    ("vercel-ai-gateway", "zai/glm-5.3"),
    ("xai", "grok-4.7"),
    ("xiaomi", "mimo-v2.6-pro"),
    ("xiaomi-token-plan-ams", "mimo-v2.6-pro"),
    ("xiaomi-token-plan-cn", "mimo-v2.6-pro"),
    ("xiaomi-token-plan-sgp", "mimo-v2.6-pro"),
    ("zai", "glm-5.3"),
    ("zai-coding-cn", "glm-5.3"),
];
