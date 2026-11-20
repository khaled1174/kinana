# Kinana 0.2.0-beta

## العربية

المصدر العام لقطة من 347c54d؛ الكود مطابق للبناء الموقّع.

كنانة مخزن محلي لمهارات الوكلاء على macOS. استورد المهارات من مجلدات أو أرشيفات ZIP أو مصادر GitHub، وراجع نتائج الفحص قبل تفعيل ما تحتاجه في كل مشروع.

### ما الجديد

- معالج أول تشغيل لاختيار المخزن واكتشاف مجلدات الوكلاء، ومعاينة تميّز المهارات الجديدة والنسخ المكررة والمختلفة قبل الاستيراد.
- تصنيف فردي وجماعي عبر Claude Code بعد عرض التقدير والتأكيد، ومراجعة الأوصاف المقترحة قبل اعتمادها. يبقى التصنيف المحلي بالكلمات المفتاحية متاحًا.
- تفعيل بالربط أو النسخ، ومعاينة قبل الكتابة، وسجل Git محلي للتراجع. يساعدك الموجّه على البحث والاقتراح من جلسة الوكيل، ويستخدم الـCLI داخل التطبيق.
- حزمة Apple Silicon (`arm64`) موقّعة وموثّقة من Apple. أدنى إصدار في الحزمة macOS 27.0. جرى التحقق من الحزمة على macOS 27.2.1. [التثبيت والخصوصية والإزالة](README.md)

### مشكلات معروفة

- **فحص الأمان أداة مساعدة لا ضمان.** في هذا الإصدار قد يصنّف مهارات عادية كخطرة بسبب متغيرات الكود والصدفة، وقد لا يتعرف على بعض صيغ المفاتيح والتوكنات. التحسين مخطط في 0.3.
- **Codex غير مدعوم للتصنيف.** يعمل معه الموجّه وتفعيل المهارات. للتصنيف، استخدم Claude Code المثبّت محليًا أو الكلمات المفتاحية.
- ما زال القبول على macOS 27 نظيف، والقبول الحي للتصنيف وبوابة `init` من Finder، والتحقق من قاعدة صلاحيات Claude Code لأوامر القراءة في انتظار النتيجة. التجارب من تطبيق فُتح من الطرفية لا تغطي بيئة Finder.
- إذا تعذّر العثور على Claude أو رفضت بوابة العزل الجلسة، اعتمد على السبب الذي تعرضه نافذة التصنيف. لا يرث تطبيق Finder مفاتيح المزود أو إعدادات البروكسي من الصدفة. لم تُختبر صيغ حقيقية جديدة لحد الاستخدام، وتقدير التوكنات تقريبي.
- ضوابط Cursor اليدوية والثانوية غير متحققة، وCursor CLI خارج الدعم. مجلد مهارات وكيل يكون رابطًا رمزيًا قد يظهر محجوبًا. توجد قيود منخفضة الأثر مع صدفة fish واقتباس المسارات وتهيئة Git وحالات استعادة `doctor --repair`.
- تنبيهات اعتماديات أدوات التطوير والاختبار وفحص توافق WDIO10 بنود مفتوحة في الجاهزية. التحديث التلقائي والواجهة الزجاجية مؤجّلان.

## English

The public source is a snapshot of 347c54d. The application code matches the signed build.

Kinana is a local Agent Skills vault for macOS. Import skills from folders, ZIP archives, or GitHub, review audit findings, and activate the skills you choose for each project.

### Highlights

- First-run vault setup and agent folder discovery, with a preview that identifies new, duplicate, and divergent skills before import.
- Individual and batch classification through Claude Code after an estimate and confirmation. Review proposed descriptions before adopting them. Local keyword classification is also available.
- Link or copy skills into projects, preview writes, and undo vault changes through local Git history. The router previews suggestions in an agent session and uses the CLI inside the app.
- A signed and Apple-notarized Apple Silicon (`arm64`) DMG. The package requires macOS 27.0 or later and was verified on macOS 27.2.1. [Installation, privacy, and removal](README.md)

### Known issues

- **The security audit is an aid, not a guarantee.** This version may flag ordinary skills because of code or shell variables. It may also miss some key and token formats. An improvement is planned for 0.3.
- **Codex classification is unsupported.** Codex can use the router and project skill activation. Classify with the locally installed Claude Code or with keywords.
- Acceptance on a clean macOS 27 system, live classification and its `init` gate from Finder, and the narrow Claude Code read-only permission rule still await results. Terminal-launched tests do not cover Finder’s environment.
- The classification dialog reports why Claude was unavailable or why its tool gate stopped a run. Finder does not inherit shell-provided provider keys or proxy settings. New live rate-limit responses have not been verified, and token estimates are approximate.
- Cursor manual and secondary controls remain unverified, and Cursor CLI is unsupported. Symlinked agent skill folders may appear blocked. Low-impact limitations affect fish shell quoting, Git configuration, and some interrupted repair cases.
- Development and test dependency advisories and WDIO10 compatibility remain open in the readiness report. Automatic updates and the redesigned glass interface are deferred.
