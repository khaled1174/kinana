# كنانة 0.3.0-beta

## الجديد في r7

- **إصلاح أمني في الفحص.** يُعرف اسم المتغير السري الآن بتقسيمه على `_` و`-` والأرقام وحدود camelCase، وبكلمات تُطابَق داخله. كانت مراجع مثل `$apiKey` و`process.env.githubToken` و`${GITHUBPAT}` تُستثنى على أنها غير سرية، فصارت «مرتفع المخاطر» كما في قواعد 0.2. قد ترتفع مهارة تقرأ اسمًا مثل `sessionId` أو `stageKey`. الفحوص المحفوظة تبقى كما هي حتى «تحديث الفحص» (الإعدادات ← الصيانة)، وتعرض معاينته ما سيتغير.
- **`kinana --json`** يحمل ما اعتمدته من حقول (العقد 11): `auto_approved` في نتيجة `classify` بدل تنبيه `AUTO_APPROVED`، و`model_default` في تأكيد Pi، و`forms` في أسباب الأسرار. ويذكر الناتج النصي الاعتماد التلقائي بسطر «Approved automatically».

## الجديد في r6

- يشمل الاعتماد التلقائي للأوصاف المهارات «النظيفة» و«التي تحتاج مراجعة»، وتبقى «مرتفعة المخاطر» وحدها بانتظار مراجعتك. الخيار في الإعدادات صار «اعتماد الأوصاف تلقائيًا للمهارات غير مرتفعة المخاطر».
- إعادة تصنيف مهارة لها وصف معتمد لا تعيدها إلى «ينتظر المراجعة» إذا طابق الوصف المقترح المعتمد بعد تطبيع المسافات؛ تظهر «اكتمل؛ الوصف الحالي لم يتغير». وإن اختلف يُعتمد تلقائيًا بالقاعدة نفسها.
- يجري الاعتماد التلقائي أيضًا بعد تصنيف لم يغيّر أي ملف، كأن يُعاد تصنيف مهارات تنتظر بالاقتراح نفسه بعد تفعيل الخيار.
- سبب الفحص الذي له موضع واحد يُعرض سطرًا واحدًا، بلا عنوان مجموعة يكرر القاعدة والملف.

## الجديد في r5

- تقرأ إعدادات Pi المزوّدين والموديلات من Pi نفسه، بأمر قراءة لا يرسل نصًا ولا يستدعي نموذجًا، وبفحوص التصنيف نفسها. الموديل اختياري: «افتراضي Pi» يمرّر موديل Pi 1.5.2 الافتراضي للمزوّد صراحة، ويظهر اسمه في نافذة التأكيد ويربطه رمزها. إذا تعذّرت القراءة يظهر السبب وحقلان نصيان.
- بعد التصنيف تُعتمد الأوصاف المقترحة للمهارات النظيفة في الفحص تلقائيًا في commit واحد، مع تنبيه «اعتُمد N وصفًا تلقائيًا · عرض · تراجع». مهارات «تحتاج مراجعة» و«مرتفع المخاطر» والوصف المعتمد سابقًا تبقى بانتظار مراجعتك. يمكن إطفاء ذلك من الإعدادات.
- حين لا تبقى مهارة غير مصنّفة يظهر «كل المهارات مصنّفة ✓» بدل الزر.
- تعرض أسباب الأسرار اسم المتغير أو الصيغة المطابقة (`$CODEX_HOME`، `os.environ.get("X")`)، وتُحجب القيم الحرفية فقط. وتُجمع الأسباب حسب القاعدة والملف، مرتفعة المخاطر أولًا، وتتوسع إلى أسطرها. لرؤية الأسماء في فحص محفوظ سابقًا: الإعدادات ← الصيانة ← تحديث الفحص.

## إصلاحات r4

- يُحفظ اختيار موديل Claude وجهده من زر بجانب الحقلين، بعد المعاينة والتأكيد. يظهر تحتهما ما إذا كان الاختيار محفوظًا، وما الذي يستخدمه التصنيف الآن، وتنتظر أزرار التصنيف في الإعدادات حتى يُحفظ.
- تظهر في نتائج التصنيف أسماء المهارات المتخطاة في سطر واحد، والسبب تحت كل اسم، في العربية والإنجليزية ومن عرض 800 بكسل.
- تعرض نافذة التصنيف الوقت المنقضي في المهارة الحالية مقابل حدها، دقيقتين للمهارة. «إلغاء» يوقف الاستدعاء الجاري فورًا.
- تظهر أسباب الفحص بالملف والسطر والمقتطف تحت سطر «الفحص» في تفاصيل المهارة. تُفتح تلقائيًا للمهارة مرتفعة المخاطر.

## الجديد في r3

- صنّف المهارات التي تنتظر التصنيف من المخزن أو الإعدادات. زر تصنيف الجميع يوضح أنه يعيد التصنيف للكل.
- اختر موديل Claude وجهده في الإعدادات وراجعهما في معاينة التكلفة. يُعرض الرفض الصريح، ويمكن تصنيف مهارة متخطّاة واحدة بالكلمات المفتاحية بعد التأكيد.
- تعلّم إعدادات التصنيف أين تختار مزوّد ChatGPT/Codex في Pi.
- دقة أعلى لمراجع البيئة غير السرية وقوالب JavaScript وأنماط .gitignore. تعرض أسباب الخطر موقعها ومقتطفًا آمنًا، وتظهر أوامر الحزم غير المقيّدة كمراجعة.

## الجديد

- **التصنيف الجماعي.** عند تجاوز مهلة مهارة أو استمرار الرد غير الصالح بعد محاولتين، تُعرض المهارة مع سبب تخطّيها ويستمر تصنيف الباقي. تعرض نافذة التوقف أيضًا سبب إخفاق المزوّد.
- **مراجعة الأوصاف دفعة واحدة.** راجع الأوصاف المقترحة واعتمد المحدد منها في عملية واحدة، مع إمكانية التراجع.
- **استيراد مهارات المشروع.** عند اختيار مشروع يحتوي على مهارات محلية في مسارات مدعومة، اختر «راجع وأضف» لفتح التدقيق ومعالجة التكرار واختيار النسخ. الاستيراد يتطلب تأكيدك ويترك ملفات المشروع كما هي. تظهر النسخ المطابقة مرتبطة بمعرّفها في المخزن.
- **تدقيق أدق.** تحسّن تمييز الإشارات السياقية عن الأنماط الخطرة، مع الاحتفاظ بمراجعة الحالات التي تتطلب قرارك.
- **Codex في إعدادات التصنيف.** يظهر معطّلًا مع السبب، لأن كنانة لا تملك ضمانًا يمنعه من قراءة ملفات الجهاز أثناء التصنيف.
- **Pi تجريبي.** مطفأ افتراضيًا. تفعيله واستخدامه للتصنيف يحتاجان اختيارًا صريحًا. يتطلب الإصدار 1.5.2؛ 1.6.0 قيد المراجعة.

## مشكلات معروفة

- تفعيل مهارة مرتفعة المخاطر في مشروع لا يطلب تأكيدًا إضافيًا؛ راجع أسباب الفحص قبل التفعيل.
- Pi تجريبي، ويتطلب الإصدار 1.5.2؛ 1.6.0 قيد المراجعة.
- قائمة Pi تُقرأ بإصدار Pi المراجَع 1.5.2 وحده؛ مع إصدار آخر يظهر السبب ويُكتب المزوّد والموديل يدويًا. «افتراضي Pi» هو الافتراضي المضمّن في Pi 1.5.2، لا الافتراضي في إعداداتك داخل Pi.
- تحسين توجيه الوصف وحد الطول المقترح ينتظران تقييم التصنيف. خيار جهد Claude يتطلب إصدار Claude Code يعرضه في المساعدة.

- اختيار الوكيل مباشرة قبل التصنيف بعد المسح، وتحرير الوصف المقترح قبل اعتماده، مؤجلان.
- شاشة تصفّح التصنيفات، وشرح سبب التشابه في كل بطاقة زوج، ولوحة انتشار المهارات في المشاريع، وتوضيح شارة «لم يُفعّل بعد»، مؤجلة.
- قد يرفع التدقيق مهارة عادية إلى «خطرة» عند قراءة متغيرات البيئة، حتى عندما تكون القيمة المقروءة غير سرية. يعرض السبب الآن اسم المتغير وصيغته.
- يستخدم التصنيف القالب 1. ينتظر القالب 2 نتيجة تقييم التصنيف المعتمد.
- سبب تخطي المهارة يظهر في نافذة النتائج فقط، ولا يُحفظ بعد إغلاقها.

يُرفق مع الإصدار ملف `THIRD_PARTY_NOTICES.md` بالإضافة إلى حزمة التطبيق وملف التحقق `SHA256.txt`.

---

# Kinana 0.3.0-beta

## What's new in r7

- **A security fix in the audit.** A secret variable name is now found by splitting the name on `_`, `-`, digits and camelCase boundaries, and by words matched inside it. References such as `$apiKey`, `process.env.githubToken` and `${GITHUBPAT}` were treated as references to names that are not secret; they are high risk again, as under the 0.2 rules. A skill that reads a name such as `sessionId` or `stageKey` may move up. Stored audits stay as they are until **Refresh audit** (Settings › Maintenance), whose preview shows what changes.
- **`kinana --json`** carries the fields you approved (contract 11): `auto_approved` in the `classify` result instead of an `AUTO_APPROVED` warning, `model_default` in the Pi confirmation, and `forms` in secrets reasons. The text output names the automatic approval on an “Approved automatically” line.

## What's new in r6

- Automatic approval of descriptions now covers skills that are clean or need review; only high-risk skills wait for your review. The setting reads “Approve descriptions automatically for skills that are not high risk”.
- Reclassifying a skill with an approved description no longer sends it back to “awaiting review” when the proposed description matches the approved one after whitespace normalization; it shows “Completed; the current description is unchanged”. A different proposal is approved automatically under the same rule.
- The automatic approval also runs after a classification that changed no file, for example when skills waiting for review are classified again with the same proposal after turning the setting on.
- An audit reason with a single place is shown as one line, without a group heading that repeats the rule and the file.

## What's new in r5

- Pi's settings read the providers and models from Pi itself, with a read-only command that sends no text and calls no model, behind the same checks as classification. The model is optional: “Pi default” passes Pi 1.5.2's default model for the provider explicitly; the confirmation shows its name and its token binds it. If the list cannot be read, the reason appears with two text fields.
- After classification, the proposed descriptions of skills that are clean in the audit are approved automatically in one commit, with “N descriptions approved automatically · Show · Undo”. Skills that need review or are risky, and descriptions approved before, wait for your review. You can turn this off in Settings.
- When no skill is left unclassified, “All skills are classified ✓” replaces the button.
- Secrets reasons show the variable name or the matching form (`$CODEX_HOME`, `os.environ.get("X")`); only literal values are hidden. Reasons are grouped by rule and file, risky first, and expand to their lines. To see the names in an audit stored earlier: Settings › Maintenance › Refresh audit.

## Fixes in r4

- Save the Claude model and effort with the button next to them, after the usual preview and confirmation. Below the fields, Settings shows whether the choice is saved and what classification uses now, and its classify buttons wait until you save.
- Classification results show each skipped skill's name on one line with its reason below, in Arabic and English, from an 800-pixel-wide window.
- The classification window shows the time spent on the current skill against its two-minute limit. **Cancel** stops the running call at once.
- Skill details list each audit reason with its file, line and excerpt under the audit line. They open automatically for a high-risk skill.

## What's new in r3

- Classify unclassified skills from the vault or Settings. Classify all explicitly reclassifies the entire vault.
- Choose Claude's model and effort in Settings and review them in the cost estimate. The dialog identifies explicit Claude refusals and lets you confirm keyword classification for one skipped skill.
- Classification settings explain where to choose the ChatGPT/Codex provider in Pi.
- Auditing distinguishes static nonsecret environment reads, JavaScript templates and .gitignore entries. Risk findings include locations and safe excerpts. Unpinned package commands are marked for review.

## What’s new

- **Batch classification.** When a skill times out or gets an invalid reply twice, Kinana shows which skill was skipped and why, then continues with the rest. If a provider stops the batch, the dialog shows the provider’s reason.
- **Batch description review.** Review proposed descriptions, approve the ones you select in a single operation, and undo that operation if needed.
- **Project skill imports.** When a selected project has skills in supported locations, choose **Review and add** to audit them, review duplicates, and select versions. Import requires confirmation and leaves project files intact. Exact matches show their vault skill ID.
- **More precise auditing.** The audit better distinguishes contextual signals from risky patterns while keeping cases that require a decision visible.
- **Codex appears disabled in classification settings.** Kinana cannot guarantee that Codex will avoid reading local files during classification.
- **Experimental Pi support.** Pi is off by default and requires an explicit choice to enable and use. It requires version 1.5.2; 1.6.0 is under review.

## Known issues

- Activating a high-risk skill in a project asks for no extra confirmation; review the audit reasons before you activate it.
- Pi is experimental and requires version 1.5.2; 1.6.0 is under review.
- Pi's list is read only with the reviewed Pi 1.5.2; with another version the reason appears and you type the provider and model. “Pi default” is Pi 1.5.2's built-in default, not the default in your own Pi settings.
- Description guidance and the proposed length target await the classification evaluation. Claude effort selection requires a Claude Code version that lists the option in help.

- Choosing the classifier immediately after a scan and editing proposed descriptions before approval are deferred.
- A category browser, explanations for suggested skill pairs, a skill-coverage dashboard, and clearer wording for the “Not activated yet” badge are deferred.
- An otherwise ordinary skill can be marked risky for reading an environment variable, even when the value is not secret. The reason now shows the variable's name and form.
- Classification uses prompt template 1. Template 2 is pending the agreed classification evaluation.
- A skipped skill's reason appears only in the results window and is not kept after you close it.

The release includes `THIRD_PARTY_NOTICES.md`, the app disk image, and `SHA256.txt`.
