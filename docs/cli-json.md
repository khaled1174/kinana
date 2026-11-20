# عقد مخرجات CLI بصيغة JSON

هذا عقد الأوامر المنفذة حتى المرحلة6.2 (الجزء المنفذ) وفق §4 من SPEC.md. تعتمد عليه مهارة الموجّه والواجهة. الأوامر المخططة لمراحل لاحقة ليست جزءًا منه. تغيير مفاتيح أو أنواع هنا يتطلب تعديلًا صريحًا للعقد واختباراته.

مع `--json` تكتب العملية قيمة JSON واحدة إلى stdout ثم سطرًا جديدًا، دون نص بشري قبله أو بعده. قد تكتب تحذيرًا إلى stderr، بما فيه تحذير الرجوع إلى الكلمات المفتاحية عند فشل Claude. افحص رمز الخروج قبل استخدام التقرير. ترتيب مفاتيح الكائنات غير مضمون؛ ترتيب القوائم تابع لمعنى الأمر. المسارات والتجزئات والتواريخ والمعرّفات ورسائل العرض قيم متغيرة، وتُمثل بنصوص. لا تستنتج الملكية من قيمة مسار أو تجزئة في التقرير.

| الأمر مع `--json` | مخطط الناتج في `$defs` |
| --- | --- |
| `migrate` / `migrate --rollback` | `Migration` |
| `init` | `Init` |
| `init --install-router` | `Init` مع `router` |
| `scan` | `Scan` |
| `scan --import` | `Import` |
| `add` | `Import` |
| `show` | `Show` |
| `search` | `Search` |
| `suggest` | `Suggest` |
| `classify` | `Classify` |
| `review` | `Review` |
| `build` | `Build` |
| `groups` | `Groups` |
| `use` | `Use` |
| `status` | `Status` |
| `unuse` | `Unuse` |
| `adopt` | `Adopt` |
| `bundle create` | `BundleSave` |
| `bundle edit` | `BundleSave` |
| `bundle show` | `Bundle` |
| `doctor` | `Doctor` |
| `doctor --rebuild-index` | `Doctor` |
| `undo` | `Undo` |
| `help` | `Help` |
| `--help` | `Help` |
| `--version` | `Version` |
| `category` | `Edit` |
| `dimensions` | `Dimensions` |
| `pin` | `Pin` |
| `remove` | `Remove` |
| `bundle delete` | `BundleDelete` |
| `list` | `قائمة من Skill` |
| `list --brief` | `BriefCatalog` |
| `bundle list` | `قائمة من Bundle` |

الأعلام والفلاتر و`--dry-run` لا تغيّر مخطط الأمر، مع استثناءين موثّقين: `scan --import`، و`list --brief` الذي يعيد كائن `BriefCatalog` بدل قائمة (قسما «عقد إصدار7» و«عقد إصدار8» أدناه). `doctor --rebuild-index` يضيف الحقل الاختياري `rebuild`. `list` و`bundle list` قائمتان في الجذر، وليستا كائنين بغلاف عام. `--help` لأي أمر و`kinana --json help <command>` يعيدان `{ "help": "..." }`، و`--version` يعيد `{ "version": "..." }`؛ محتوى النص قابل للتغير. مع أمر `help` المدمج في clap ضع `--json` قبله؛ ما يأتي بعده يُفسر كمسار أمر للمساعدة.

| رمز الخروج | stdout مع `--json` |
| --- | --- |
| `0` | التقرير الموثق، بما فيه المعاينة وعمليات بلا تغيير. |
| `1` | `Error` لخطأ تنفيذ أو خيارات غير صالحة (`INVALID_ARGUMENTS` لأخطاء parser). استثناء: `doctor` عند اكتمال التشخيص ووجود فحص فاشل يعيد `Doctor` مع `healthy: false`، بلا غلاف `error`. |
| `2` | `Error` عند الحاجة إلى قرار صريح أو وجود تعارض، مثل تأكيد التصنيف الجماعي أو تعارض هدف التفعيل. |

شكل الخطأ حرفيًا من حيث المفاتيح والأنواع: `{"error":{"code":"string","reason_code":"string","message":"string"}}`. الحقل الداخلي `needs_decision` لا يُرسل؛ رمز الخروج يميز القرار. `code` رمز قابل للمعالجة و`reason_code` سبب ثابت لترجمة الواجهة (يساوي code افتراضيًا، وقد يفصّل IO_PERMISSION_DENIED مثلًا)، و`message` للتوضيح التشخيصي؛ لا تعتمد على نص الرسالة أو تعرضه مباشرة في الواجهة العربية. رموز فشل النظام قبل بدء العملية أو الإنهاء بإشارة خارج هذا العقد. إذا تعذرت الكتابة إلى stdout فقد لا يصل تقرير كامل.

تقييم `doctor` وإعادة البناء مستقلان: معاينة إعادة بناء صالحة قد تنجح برمز0 رغم `healthy: false` للفهرس الحالي؛ التنفيذ قد يعيد `rebuilt: true` ورمز1 إذا بقي عطل في مشروع. تحقّق من الحقلين، لا من أحدهما وحده.

`category <id> <category>` يضبط تصنيفًا من box.toml دون اعتماد الوصف أو حذف اقتراحه، ويقبل المعاينة. `list --query <text>` يطبق بحث core الحرفي ثم مرشحات القائمة، و`list --state` يقبل pending/proposal/approved/original؛ شكل الناتج نفسه.

`list --stack <value>` و`list --stage <value>` يعرضان المهارات التي تحمل القيمة، ويجتمعان مع `--category` وبقية المرشحات (كلها مطلوبة معًا)؛ شكل الناتج نفسه. قيمة ليست في قائمتها في box.toml خطأ `INVALID_STACK` / `INVALID_STAGE` برمز1، وقيمة صحيحة لا تحملها أي مهارة تعيد `[]`. تفاصيل الأبعاد في قسم «أبعاد التصنيف — عقد إصدار6» أدناه.

`pinned` حاضر دائمًا كـBoolean في Skill وMetadata، ويعني false عند غيابه من TOML القديم دون كتابة. `pin --unset` يضبط false، والتكرار بلا تغيير. pin لا يغيّر dist أو skills أو الفهرس.

توسعة Cursor قبل المرحلة7 لا تغيّر مفاتيح أو أنواع العقد: حقول agent وAgent.name قد تحمل `cursor`، ومساره الجديد `.agents/skills` المشترك مع Codex، و`--agent all` في التفعيل/الإلغاء/adopt يشمل الوكلاء الثلاثة. `list --agent cursor` مدعوم. التصنيف عبر Cursor غير مدعوم ويرفضه parser، وتثبيت الموجّه العام يبقى للوجهتين المعتمدتين فقط. اعتمد المالك إضافة outdated/update. الإصدار3 يضيف عقديهما صراحةً والحقول الاختيارية أدناه.

تعديل6.4 المعتمد من المالك: Audit يحافظ على findings النصية ويضيف rules_version (الإصدار الجديد2، والقديم0) وdetails المهيكلة، مع سبب ثابت وقوة وموقع لكل مطابقة. القراءة القديمة لا تكتب metadata؛ لا تُستنتج clean من غياب التفاصيل. يتغير عقد المخطط صراحةً إلى2 وتختبر الأوامر الشكل الجديد.

جميع الكائنات في المخطط مغلقة (`additionalProperties: false`). الحقول المذكورة في `required` إلزامية دائمًا. الحقول الأخرى يجوز حذفها فقط، وإذا حضرت وجب أن تطابق نوعها. `null` مسموح فقط حيث ينص المخطط عليه؛ القوائم الفارغة تبقى `[]`.

- `Metadata.proposal` و`empty_directories` يُحذفان عند غيابهما/فراغهما. `Classification.classified_from` يُحذف للبيانات القديمة/غير المصنفة بالنموذج؛ عند وجوده قيمته `full` أو `excerpt`. `overlaps_with` يُحذف إذا كان فارغًا. `Classification.stack` و`Classification.stage` و`Proposal.stack` و`Proposal.stage` تُحذف إذا كانت فارغة أو غائبة عن TOML القديم، لأن Metadata مرآة لملف meta الذي لا يُكتب فيه البُعد الفارغ؛ القيمة الحاضرة دائمًا هي `Skill.stack` و`Skill.stage`.
- `Description.improved` و`approved_at` و`Review.proposed` حقول حاضرة تقبل `null`، وكذلك حقول المصدر الاختيارية و`ClassifiedItem.classified_from` و`Estimate.calibration` و`Rebuild.backup` و`Adopt.activation` و`Observation.lock`.
- `BundleSkill.group` و`Lock.bundle` و`Lock.restoration` و`LockedSkill.ownership` حقول قد تُحذف، خصوصًا في المعاينة/البيانات القديمة. هوية الملكية نص معتم.
- `Restoration.original_lock_mode` و`original_empty_lock` يظهران فقط عند الحاجة لاستعادة قفل فارغ سابق. أسماء المجلدات وهوياتها في `directories` لا تعطي إذن حذف لعميل CLI.
- `Classify.estimated_tokens` يفصل الثابت لكل استدعاء ومجموعه عن نص المهارات؛ `calibration` يحمل قياس usage المرجعي عند استعمال تقدير Claude، وإلا `null`. `prompt_sha256` ليس حقلًا في ناتج CLI.


إضافة3 المعتمدة: `outdated` يجمع حسب المستودع والمرجع ونوعه، بسحب واحد للمجموعة، ويعيد Outdated برمز0 إذا complete=true وبرمز1 عند أخطاء العناصر مع بقائها في التقرير. الحالات: unchanged، source_advanced، outdated، tracking_required، unsupported_source، error. complete تعني اكتمال الفحص دون خطأ؛ tracking_required وunsupported_source حالتا معلومة دون سحب، ولا تعنيان أن المهارة محدثة.

`update <id>` يعيد Update بمعاينة كاملة ورمز2 إذا requires_confirmation=true؛ `--dry-run` يعيد المعاينة برمز0 دون كتابة. بعد المراجعة يُكرر الأمر مع `--confirm <preview_token>` (و`--allow-risky` عند الحاجة). النجاح الفعلي أو عدم الحاجة لتغيير رمز0. الرفض/التعارض Error برمز2، وفشل التنفيذ/المصدر Error برمز1. `--ref` يقبل اسم فرع أو branch:name أو tag:name أو commit:SHA أو default؛ يُحفظ بعد التأكيد فقط. --yes وحده مرفوض ولا يعوض التأكيد. preview_token معتم، لا يُنشئ العميل رمزه أو يستنتج صلاحيته من شكله.

FileDiff يعلن نوع وحجم وصلاحيات وتجزئة الطرفين، وunified_diff للنص المعروض. diff_byte_limit=524288، والملف النصي الذي يتجاوز مجموع نصه القديم والجديد65536 بايت لا يُعرض نصه، مع truncated=true وزيادة omitted_files؛ تبقى تجزئاته ومعلوماته. null في unified_diff قد يعني ثنائيًا أو تغيير صلاحية/نوع بلا فرق نصي، أو نصًا لم يُعرض؛ truncated يميز الحذف بسبب الحد.

Metadata.proposal_source_changed اختياري، ويعلّم اقتراحًا قائمًا تغيّر مصدره؛ يُمسح عند المراجعة أو إنشاء نتيجة تصنيف جديدة. Source.tracking_ref وtracking_kind اختياريان، والغياب يحفظ توافق metadata القديمة دون تخمين. LockedSkill.target_root اختياري ومقيد بمسار الوكيل الثابت؛ Cursor القديم دون الحقل يبقى في .cursor/skills، والجديد يحمل .agents/skills. generation_hash اختياري لتجزئة الجيل بما يشمل صلاحيات الملفات؛ غيابه يبقي تحقق القفل القديم بالمحتوى. لا يجيز أي حقل مسارًا عامًا أو حذفًا دون الملكية. تظهر ثلاثة تفعيلات منطقية عند all في وجهتين ماديتين فقط.

المخطط التالي JSON Schema Draft 2020-12. استخرج محتوى السياج بين العلامتين؛ `x-commands` يختار مخطط الأمر و`x-error` مخطط الأخطاء. `x-contract-version` نسخة هذا المستند، وليس حقلًا مضافًا إلى تقارير CLI. `x-released-in` الإصدار الذي شُحنت فيه هذه النسخة: **النسخة 8 هي عقد الإصدار 0.2.0 ومثبتة عنده** (قرار المالك، الجولة الثامنة)؛ أي تغيير في المفاتيح أو الأنواع بعده نسخة 9. الاختبار يطابق `x-released-in` مع إصدار الحزمة، فرفع الإصدار يستلزم مراجعة العقد صراحة. الاختبار `crates/cli/tests/json_contract.rs` يقرأ هذا النص نفسه، يتحقق من جرد الأوامر، ويقارن المفاتيح والأنواع على مخرجات عمليات حقيقية. لا يُولّد المخطط من نماذج Rust.

<!-- cli-json-contract:start -->
```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "x-contract-version": 8,
  "x-released-in": "0.2.0",
  "x-commands": {
    "init": {
      "$ref": "#/$defs/Init"
    },
    "pin": {
      "$ref": "#/$defs/Pin"
    },
    "remove": {
      "$ref": "#/$defs/Remove"
    },
    "bundle delete": {
      "$ref": "#/$defs/BundleDelete"
    },
    "category": {
      "$ref": "#/$defs/Edit"
    },
    "dimensions": {
      "$ref": "#/$defs/Dimensions"
    },
    "scan": {
      "$ref": "#/$defs/Scan"
    },
    "scan --import": {
      "$ref": "#/$defs/Import"
    },
    "add": {
      "$ref": "#/$defs/Import"
    },
    "show": {
      "$ref": "#/$defs/Show"
    },
    "classify": {
      "$ref": "#/$defs/Classify"
    },
    "review": {
      "$ref": "#/$defs/Review"
    },
    "build": {
      "$ref": "#/$defs/Build"
    },
    "groups": {
      "$ref": "#/$defs/Groups"
    },
    "use": {
      "$ref": "#/$defs/Use"
    },
    "status": {
      "$ref": "#/$defs/Status"
    },
    "unuse": {
      "$ref": "#/$defs/Unuse"
    },
    "adopt": {
      "$ref": "#/$defs/Adopt"
    },
    "bundle create": {
      "$ref": "#/$defs/BundleSave"
    },
    "bundle edit": {
      "$ref": "#/$defs/BundleSave"
    },
    "bundle show": {
      "$ref": "#/$defs/Bundle"
    },
    "doctor": {
      "$ref": "#/$defs/Doctor"
    },
    "doctor --rebuild-index": {
      "$ref": "#/$defs/Doctor"
    },
    "doctor --repair": {
      "$ref": "#/$defs/DoctorRepair"
    },
    "undo": {
      "$ref": "#/$defs/Undo"
    },
    "help": {
      "$ref": "#/$defs/Help"
    },
    "--help": {
      "$ref": "#/$defs/Help"
    },
    "--version": {
      "$ref": "#/$defs/Version"
    },
    "list": {
      "type": "array",
      "items": {
        "$ref": "#/$defs/Skill"
      }
    },
    "list --brief": {
      "$ref": "#/$defs/BriefCatalog"
    },
    "bundle list": {
      "type": "array",
      "items": {
        "$ref": "#/$defs/Bundle"
      }
    },
    "search": {
      "$ref": "#/$defs/Search"
    },
    "suggest": {
      "$ref": "#/$defs/Suggest"
    },
    "init --install-router": {
      "$ref": "#/$defs/Init"
    },
    "outdated": {
      "$ref": "#/$defs/Outdated"
    },
    "update": {
      "$ref": "#/$defs/Update"
    },
    "migrate": {
      "$ref": "#/$defs/Migration"
    },
    "migrate --rollback": {
      "$ref": "#/$defs/Migration"
    }
  },
  "x-error": {
    "$ref": "#/$defs/Error"
  },
  "$defs": {
    "Edit": {
      "type": "object",
      "properties": {
        "changed": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "built": {
          "$ref": "#/$defs/Build"
        },
        "committed": {
          "type": "boolean"
        }
      },
      "required": [
        "changed",
        "built",
        "committed"
      ],
      "additionalProperties": false
    },
    "Pin": {
      "type": "object",
      "properties": {
        "id": {
          "type": "string"
        },
        "pinned": {
          "type": "boolean"
        },
        "dry_run": {
          "type": "boolean"
        },
        "changed": {
          "type": "boolean"
        },
        "committed": {
          "type": "boolean"
        }
      },
      "required": [
        "id",
        "pinned",
        "dry_run",
        "changed",
        "committed"
      ],
      "additionalProperties": false
    },
    "Dimensions": {
      "type": "object",
      "properties": {
        "id": {
          "type": "string"
        },
        "stack": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "stage": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "dry_run": {
          "type": "boolean"
        },
        "changed": {
          "type": "boolean"
        },
        "committed": {
          "type": "boolean"
        }
      },
      "required": [
        "id",
        "stack",
        "stage",
        "dry_run",
        "changed",
        "committed"
      ],
      "additionalProperties": false
    },
    "Remove": {
      "type": "object",
      "properties": {
        "id": {
          "type": "string"
        },
        "dry_run": {
          "type": "boolean"
        },
        "changed": {
          "type": "boolean"
        },
        "committed": {
          "type": "boolean"
        },
        "removed_paths": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "bundles_changed": {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "id",
        "dry_run",
        "changed",
        "committed",
        "removed_paths",
        "bundles_changed"
      ],
      "additionalProperties": false
    },
    "BundleDelete": {
      "type": "object",
      "properties": {
        "name": {
          "type": "string"
        },
        "dry_run": {
          "type": "boolean"
        },
        "changed": {
          "type": "boolean"
        },
        "committed": {
          "type": "boolean"
        }
      },
      "required": [
        "name",
        "dry_run",
        "changed",
        "committed"
      ],
      "additionalProperties": false
    },
    "Source": {
      "type": "object",
      "properties": {
        "kind": {
          "type": "string"
        },
        "url": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "subpath": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "commit": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "found_in": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "tracking_ref": {
          "type": "string"
        },
        "tracking_kind": {
          "type": "string"
        }
      },
      "required": [
        "kind",
        "url",
        "subpath",
        "commit",
        "found_in"
      ],
      "additionalProperties": false
    },
    "Classification": {
      "type": "object",
      "properties": {
        "category": {
          "type": "string"
        },
        "role": {
          "type": "string"
        },
        "when_to_use": {
          "type": "string"
        },
        "when_not": {
          "type": "string"
        },
        "classified_by": {
          "type": "string"
        },
        "tags": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "classified_from": {
          "type": "string"
        },
        "overlaps_with": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "stack": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "stage": {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "category",
        "role",
        "when_to_use",
        "when_not",
        "classified_by",
        "tags"
      ],
      "additionalProperties": false
    },
    "Description": {
      "type": "object",
      "properties": {
        "original": {
          "type": "string"
        },
        "improved": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "active": {
          "type": "string"
        },
        "approved_at": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        }
      },
      "required": [
        "original",
        "improved",
        "active",
        "approved_at"
      ],
      "additionalProperties": false
    },
    "Audit": {
      "type": "object",
      "properties": {
        "level": {
          "type": "string"
        },
        "findings": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "rules_version": {
          "type": "integer"
        },
        "details": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/AuditFinding"
          }
        }
      },
      "required": [
        "level",
        "findings",
        "rules_version",
        "details"
      ],
      "additionalProperties": false
    },
    "AuditFinding": {
      "type": "object",
      "properties": {
        "rule": {
          "type": "string"
        },
        "strength": {
          "type": "string"
        },
        "level": {
          "type": "string"
        },
        "reason_code": {
          "type": "string"
        },
        "path": {
          "type": "string"
        },
        "line": {
          "anyOf": [
            {
              "type": "integer"
            },
            {
              "type": "null"
            }
          ]
        },
        "end_line": {
          "anyOf": [
            {
              "type": "integer"
            },
            {
              "type": "null"
            }
          ]
        },
        "excerpt": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        }
      },
      "required": [
        "rule",
        "strength",
        "level",
        "reason_code",
        "path",
        "line",
        "end_line",
        "excerpt"
      ],
      "additionalProperties": false
    },
    "Proposal": {
      "type": "object",
      "properties": {
        "category": {
          "type": "string"
        },
        "role": {
          "type": "string"
        },
        "when_to_use": {
          "type": "string"
        },
        "when_not": {
          "type": "string"
        },
        "improved_description": {
          "type": "string"
        },
        "tags": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "overlaps_with": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "confidence": {
          "type": "number"
        },
        "stack": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "stage": {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "category",
        "role",
        "when_to_use",
        "when_not",
        "improved_description",
        "tags",
        "overlaps_with",
        "confidence"
      ],
      "additionalProperties": false
    },
    "Metadata": {
      "type": "object",
      "properties": {
        "id": {
          "type": "string"
        },
        "content_hash": {
          "type": "string"
        },
        "added_at": {
          "type": "string"
        },
        "source": {
          "$ref": "#/$defs/Source"
        },
        "classification": {
          "$ref": "#/$defs/Classification"
        },
        "description": {
          "$ref": "#/$defs/Description"
        },
        "audit": {
          "$ref": "#/$defs/Audit"
        },
        "empty_directories": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "proposal": {
          "$ref": "#/$defs/Proposal"
        },
        "pinned": {
          "type": "boolean"
        },
        "proposal_source_changed": {
          "type": "boolean"
        }
      },
      "required": [
        "id",
        "content_hash",
        "added_at",
        "source",
        "classification",
        "description",
        "audit",
        "pinned"
      ],
      "additionalProperties": false
    },
    "Skill": {
      "type": "object",
      "properties": {
        "id": {
          "type": "string"
        },
        "name": {
          "type": "string"
        },
        "description": {
          "type": "string"
        },
        "category": {
          "type": "string"
        },
        "audit_level": {
          "type": "string"
        },
        "content_hash": {
          "type": "string"
        },
        "added_at": {
          "type": "string"
        },
        "role": {
          "type": "string"
        },
        "source": {
          "$ref": "#/$defs/Source"
        },
        "tags": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "pinned": {
          "type": "boolean"
        },
        "stack": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "stage": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "when_to_use": {
          "type": "string"
        }
      },
      "required": [
        "id",
        "name",
        "description",
        "category",
        "audit_level",
        "content_hash",
        "added_at",
        "role",
        "source",
        "tags",
        "pinned",
        "stack",
        "stage",
        "when_to_use"
      ],
      "additionalProperties": false
    },
    "ScanEntry": {
      "type": "object",
      "properties": {
        "path": {
          "type": "string"
        },
        "name": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "description": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "content_hash": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "status": {
          "type": "string"
        },
        "skill_id": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "source": {
          "$ref": "#/$defs/Source"
        },
        "error": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        }
      },
      "required": [
        "path",
        "name",
        "description",
        "content_hash",
        "status",
        "skill_id",
        "source",
        "error"
      ],
      "additionalProperties": false
    },
    "ScanCounts": {
      "type": "object",
      "properties": {
        "new": {
          "type": "integer"
        },
        "duplicate": {
          "type": "integer"
        },
        "variant": {
          "type": "integer"
        },
        "broken": {
          "type": "integer"
        }
      },
      "required": [
        "new",
        "duplicate",
        "variant",
        "broken"
      ],
      "additionalProperties": false
    },
    "Scan": {
      "type": "object",
      "properties": {
        "entries": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/ScanEntry"
          }
        },
        "counts": {
          "$ref": "#/$defs/ScanCounts"
        },
        "warnings": {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "entries",
        "counts",
        "warnings"
      ],
      "additionalProperties": false
    },
    "Show": {
      "type": "object",
      "properties": {
        "skill": {
          "$ref": "#/$defs/Skill"
        },
        "metadata": {
          "$ref": "#/$defs/Metadata"
        },
        "raw_skill_md": {
          "type": "string"
        },
        "locations": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "variants": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/ScanEntry"
          }
        }
      },
      "required": [
        "skill",
        "metadata",
        "raw_skill_md",
        "locations",
        "variants"
      ],
      "additionalProperties": false
    },
    "Agent": {
      "type": "object",
      "properties": {
        "name": {
          "type": "string"
        },
        "available": {
          "type": "boolean"
        },
        "project_skills": {
          "type": "string"
        },
        "global_skills": {
          "type": "string"
        }
      },
      "required": [
        "name",
        "available",
        "project_skills",
        "global_skills"
      ],
      "additionalProperties": false
    },
    "Init": {
      "type": "object",
      "properties": {
        "vault": {
          "type": "string"
        },
        "index": {
          "type": "string"
        },
        "agents": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/Agent"
          }
        },
        "dry_run": {
          "type": "boolean"
        },
        "created": {
          "type": "boolean"
        },
        "planned_actions": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "router": {
          "$ref": "#/$defs/RouterInstall"
        }
      },
      "required": [
        "vault",
        "index",
        "agents",
        "dry_run",
        "created",
        "planned_actions"
      ],
      "additionalProperties": false
    },
    "ImportItem": {
      "type": "object",
      "properties": {
        "id": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "name": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "path": {
          "type": "string"
        },
        "status": {
          "type": "string"
        },
        "content_hash": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "audit": {
          "$ref": "#/$defs/Audit"
        },
        "selected": {
          "type": "boolean"
        },
        "error": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        }
      },
      "required": [
        "id",
        "name",
        "path",
        "status",
        "content_hash",
        "audit",
        "selected",
        "error"
      ],
      "additionalProperties": false
    },
    "Import": {
      "type": "object",
      "properties": {
        "source": {
          "type": "string"
        },
        "dry_run": {
          "type": "boolean"
        },
        "requires_preparation": {
          "type": "boolean"
        },
        "items": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/ImportItem"
          }
        },
        "imported": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "duplicates": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "variants": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "committed": {
          "type": "boolean"
        },
        "index_updated": {
          "type": "boolean"
        },
        "warnings": {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "source",
        "dry_run",
        "requires_preparation",
        "items",
        "imported",
        "duplicates",
        "variants",
        "committed",
        "index_updated",
        "warnings"
      ],
      "additionalProperties": false
    },
    "BuildItem": {
      "type": "object",
      "properties": {
        "id": {
          "type": "string"
        },
        "path": {
          "type": "string"
        },
        "description_source": {
          "type": "string"
        },
        "changed": {
          "type": "boolean"
        }
      },
      "required": [
        "id",
        "path",
        "description_source",
        "changed"
      ],
      "additionalProperties": false
    },
    "Build": {
      "type": "object",
      "properties": {
        "dry_run": {
          "type": "boolean"
        },
        "items": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/BuildItem"
          }
        },
        "removed": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "warnings": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "finder_files_removed": {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "dry_run",
        "items",
        "removed",
        "warnings"
      ],
      "additionalProperties": false
    },
    "Calibration": {
      "type": "object",
      "properties": {
        "claude_version": {
          "type": "string"
        },
        "model": {
          "type": "string"
        },
        "source_log": {
          "type": "string"
        },
        "input_tokens": {
          "type": "integer"
        },
        "cache_creation_input_tokens": {
          "type": "integer"
        },
        "cache_read_input_tokens": {
          "type": "integer"
        },
        "total_input_tokens": {
          "type": "integer"
        },
        "prompt_utf8_bytes": {
          "type": "integer"
        },
        "estimated_prompt_tokens": {
          "type": "integer"
        },
        "cli_overhead_tokens": {
          "type": "integer"
        }
      },
      "required": [
        "claude_version",
        "model",
        "source_log",
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
        "total_input_tokens",
        "prompt_utf8_bytes",
        "estimated_prompt_tokens",
        "cli_overhead_tokens"
      ],
      "additionalProperties": false
    },
    "Estimate": {
      "type": "object",
      "properties": {
        "fixed_tokens_per_call": {
          "type": "integer"
        },
        "fixed_tokens_total": {
          "type": "integer"
        },
        "skill_tokens_total": {
          "type": "integer"
        },
        "total_input_tokens": {
          "type": "integer"
        },
        "retry_limit_input_tokens": {
          "type": "integer"
        },
        "calibration": {
          "anyOf": [
            {
              "$ref": "#/$defs/Calibration"
            },
            {
              "type": "null"
            }
          ]
        }
      },
      "required": [
        "fixed_tokens_per_call",
        "fixed_tokens_total",
        "skill_tokens_total",
        "total_input_tokens",
        "retry_limit_input_tokens",
        "calibration"
      ],
      "additionalProperties": false
    },
    "ClassifyStop": {
      "type": "object",
      "properties": {
        "reason_code": {
          "type": "string"
        },
        "message": {
          "type": "string"
        },
        "classified": {
          "type": "integer"
        },
        "remaining": {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "reason_code",
        "message",
        "classified",
        "remaining"
      ],
      "additionalProperties": false
    },
    "ClassifiedItem": {
      "type": "object",
      "properties": {
        "id": {
          "type": "string"
        },
        "category": {
          "type": "string"
        },
        "classified_by": {
          "type": "string"
        },
        "proposal_created": {
          "type": "boolean"
        },
        "attempts": {
          "type": "integer"
        },
        "warnings": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "classified_from": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "estimated_skill_tokens": {
          "type": "integer"
        }
      },
      "required": [
        "id",
        "category",
        "classified_by",
        "proposal_created",
        "attempts",
        "warnings",
        "classified_from",
        "estimated_skill_tokens"
      ],
      "additionalProperties": false
    },
    "Classify": {
      "type": "object",
      "properties": {
        "dry_run": {
          "type": "boolean"
        },
        "selected": {
          "type": "integer"
        },
        "estimated_agent_calls": {
          "type": "integer"
        },
        "estimated_tokens": {
          "$ref": "#/$defs/Estimate"
        },
        "agent_calls": {
          "type": "integer"
        },
        "items": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/ClassifiedItem"
          }
        },
        "committed": {
          "type": "boolean"
        },
        "build": {
          "$ref": "#/$defs/Build"
        },
        "warnings": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "stopped": {
          "$ref": "#/$defs/ClassifyStop"
        }
      },
      "required": [
        "dry_run",
        "selected",
        "estimated_agent_calls",
        "estimated_tokens",
        "agent_calls",
        "items",
        "committed",
        "build",
        "warnings"
      ],
      "additionalProperties": false
    },
    "Review": {
      "type": "object",
      "properties": {
        "id": {
          "type": "string"
        },
        "original": {
          "type": "string"
        },
        "current": {
          "type": "string"
        },
        "proposed": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "active": {
          "type": "string"
        },
        "decision": {
          "type": "string"
        },
        "dry_run": {
          "type": "boolean"
        },
        "committed": {
          "type": "boolean"
        },
        "build": {
          "$ref": "#/$defs/Build"
        }
      },
      "required": [
        "id",
        "original",
        "current",
        "proposed",
        "active",
        "decision",
        "dry_run",
        "committed",
        "build"
      ],
      "additionalProperties": false
    },
    "BundleSkill": {
      "type": "object",
      "properties": {
        "id": {
          "type": "string"
        },
        "role": {
          "type": "string"
        },
        "group": {
          "type": "string"
        }
      },
      "required": [
        "id",
        "role"
      ],
      "additionalProperties": false
    },
    "Bundle": {
      "type": "object",
      "properties": {
        "name": {
          "type": "string"
        },
        "title": {
          "type": "string"
        },
        "agents": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "skills": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/BundleSkill"
          }
        }
      },
      "required": [
        "name",
        "title",
        "agents",
        "skills"
      ],
      "additionalProperties": false
    },
    "BundleSave": {
      "type": "object",
      "properties": {
        "dry_run": {
          "type": "boolean"
        },
        "operation": {
          "type": "string"
        },
        "bundle": {
          "$ref": "#/$defs/Bundle"
        },
        "path": {
          "type": "string"
        },
        "changed": {
          "type": "boolean"
        },
        "committed": {
          "type": "boolean"
        },
        "warnings": {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "dry_run",
        "operation",
        "bundle",
        "path",
        "changed",
        "committed",
        "warnings"
      ],
      "additionalProperties": false
    },
    "Group": {
      "type": "object",
      "properties": {
        "skill_ids": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "category": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "reasons": {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "skill_ids",
        "category",
        "reasons"
      ],
      "additionalProperties": false
    },
    "Groups": {
      "type": "object",
      "properties": {
        "candidates": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/Group"
          }
        },
        "warnings": {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "candidates",
        "warnings"
      ],
      "additionalProperties": false
    },
    "OwnedDirectory": {
      "type": "object",
      "properties": {
        "path": {
          "type": "string"
        },
        "identity": {
          "type": "string"
        }
      },
      "required": [
        "path",
        "identity"
      ],
      "additionalProperties": false
    },
    "Restoration": {
      "type": "object",
      "properties": {
        "created_lock": {
          "type": "boolean"
        },
        "original_lock_mode": {
          "type": "integer"
        },
        "original_empty_lock": {
          "type": "string"
        },
        "directories": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/OwnedDirectory"
          }
        }
      },
      "required": [
        "created_lock",
        "directories"
      ],
      "additionalProperties": false
    },
    "LockedSkill": {
      "type": "object",
      "properties": {
        "id": {
          "type": "string"
        },
        "agent": {
          "type": "string"
        },
        "mode": {
          "type": "string"
        },
        "name": {
          "type": "string"
        },
        "invocation": {
          "type": "string"
        },
        "content_hash": {
          "type": "string"
        },
        "ownership": {
          "type": "string"
        },
        "target_root": {
          "type": "string"
        },
        "generation_hash": {
          "type": "string"
        }
      },
      "required": [
        "id",
        "agent",
        "mode",
        "name",
        "invocation",
        "content_hash"
      ],
      "additionalProperties": false
    },
    "Lock": {
      "type": "object",
      "properties": {
        "vault": {
          "type": "string"
        },
        "restoration": {
          "$ref": "#/$defs/Restoration"
        },
        "bundle": {
          "type": "string"
        },
        "active": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/LockedSkill"
          }
        }
      },
      "required": [
        "vault",
        "active"
      ],
      "additionalProperties": false
    },
    "ActivationItem": {
      "type": "object",
      "properties": {
        "id": {
          "type": "string"
        },
        "agent": {
          "type": "string"
        },
        "mode": {
          "type": "string"
        },
        "invocation": {
          "type": "string"
        },
        "name": {
          "type": "string"
        },
        "source": {
          "type": "string"
        },
        "target": {
          "type": "string"
        },
        "content_hash": {
          "type": "string"
        },
        "changed": {
          "type": "boolean"
        }
      },
      "required": [
        "id",
        "agent",
        "mode",
        "invocation",
        "name",
        "source",
        "target",
        "content_hash",
        "changed"
      ],
      "additionalProperties": false
    },
    "Use": {
      "type": "object",
      "properties": {
        "project": {
          "type": "string"
        },
        "dry_run": {
          "type": "boolean"
        },
        "changed": {
          "type": "boolean"
        },
        "index_changed": {
          "type": "boolean"
        },
        "items": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/ActivationItem"
          }
        },
        "lock": {
          "$ref": "#/$defs/Lock"
        },
        "gitignore_suggestions": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "warnings": {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "project",
        "dry_run",
        "changed",
        "index_changed",
        "items",
        "lock",
        "gitignore_suggestions",
        "warnings"
      ],
      "additionalProperties": false
    },
    "ActivationStatus": {
      "type": "object",
      "properties": {
        "id": {
          "type": "string"
        },
        "agent": {
          "type": "string"
        },
        "target": {
          "type": "string"
        },
        "state": {
          "type": "string"
        },
        "message": {
          "type": "string"
        },
        "reason_code": {
          "type": "string"
        }
      },
      "required": [
        "id",
        "agent",
        "target",
        "state",
        "message",
        "reason_code"
      ],
      "additionalProperties": false
    },
    "ProjectStatus": {
      "type": "object",
      "properties": {
        "project": {
          "type": "string"
        },
        "state": {
          "type": "string"
        },
        "items": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/ActivationStatus"
          }
        },
        "warnings": {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "project",
        "state",
        "items",
        "warnings"
      ],
      "additionalProperties": false
    },
    "Status": {
      "type": "object",
      "properties": {
        "projects": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/ProjectStatus"
          }
        }
      },
      "required": [
        "projects"
      ],
      "additionalProperties": false
    },
    "Unuse": {
      "type": "object",
      "properties": {
        "project": {
          "type": "string"
        },
        "dry_run": {
          "type": "boolean"
        },
        "changed": {
          "type": "boolean"
        },
        "removed": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "removed_directories": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "lock_removed": {
          "type": "boolean"
        },
        "warnings": {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "project",
        "dry_run",
        "changed",
        "removed",
        "removed_directories",
        "lock_removed",
        "warnings"
      ],
      "additionalProperties": false
    },
    "AdoptionCandidate": {
      "type": "object",
      "properties": {
        "target": {
          "type": "string"
        },
        "agent": {
          "type": "string"
        },
        "id": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "content_hash": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "link_to": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "eligible": {
          "type": "boolean"
        },
        "reason": {
          "type": "string"
        },
        "reason_code": {
          "type": "string"
        }
      },
      "required": [
        "target",
        "agent",
        "id",
        "content_hash",
        "link_to",
        "eligible",
        "reason",
        "reason_code"
      ],
      "additionalProperties": false
    },
    "Adopt": {
      "type": "object",
      "properties": {
        "dry_run": {
          "type": "boolean"
        },
        "candidates": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/AdoptionCandidate"
          }
        },
        "activation": {
          "anyOf": [
            {
              "$ref": "#/$defs/Use"
            },
            {
              "type": "null"
            }
          ]
        }
      },
      "required": [
        "dry_run",
        "candidates",
        "activation"
      ],
      "additionalProperties": false
    },
    "Observation": {
      "type": "object",
      "properties": {
        "project": {
          "type": "string"
        },
        "status": {
          "type": "string"
        },
        "message": {
          "type": "string"
        },
        "lock": {
          "anyOf": [
            {
              "$ref": "#/$defs/Lock"
            },
            {
              "type": "null"
            }
          ]
        }
      },
      "required": [
        "project",
        "status",
        "message",
        "lock"
      ],
      "additionalProperties": false
    },
    "Rebuild": {
      "type": "object",
      "properties": {
        "vault": {
          "type": "string"
        },
        "index": {
          "type": "string"
        },
        "dry_run": {
          "type": "boolean"
        },
        "rebuilt": {
          "type": "boolean"
        },
        "mode": {
          "type": "string"
        },
        "skills": {
          "type": "integer"
        },
        "groups": {
          "type": "integer"
        },
        "projects": {
          "type": "integer"
        },
        "activations": {
          "type": "integer"
        },
        "other_vaults_preserved": {
          "type": "boolean"
        },
        "other_vaults_recovery_required": {
          "type": "boolean"
        },
        "observations": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/Observation"
          }
        },
        "warnings": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "backup": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        }
      },
      "required": [
        "vault",
        "index",
        "dry_run",
        "rebuilt",
        "mode",
        "skills",
        "groups",
        "projects",
        "activations",
        "other_vaults_preserved",
        "other_vaults_recovery_required",
        "observations",
        "warnings",
        "backup"
      ],
      "additionalProperties": false
    },
    "Check": {
      "type": "object",
      "properties": {
        "name": {
          "type": "string"
        },
        "reason_code": {
          "type": "string"
        },
        "passed": {
          "type": "boolean"
        },
        "message": {
          "type": "string"
        }
      },
      "required": [
        "name",
        "reason_code",
        "passed",
        "message"
      ],
      "additionalProperties": false
    },
    "Doctor": {
      "type": "object",
      "properties": {
        "vault": {
          "type": "string"
        },
        "index": {
          "type": "string"
        },
        "agents": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/Agent"
          }
        },
        "checks": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/Check"
          }
        },
        "healthy": {
          "type": "boolean"
        },
        "projects": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/ProjectStatus"
          }
        },
        "warnings": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "rebuild": {
          "$ref": "#/$defs/Rebuild"
        }
      },
      "required": [
        "vault",
        "index",
        "agents",
        "checks",
        "healthy",
        "projects",
        "warnings"
      ],
      "additionalProperties": false
    },
    "DoctorRepair": {
      "type": "object",
      "properties": {
        "vault": {
          "type": "string"
        },
        "dry_run": {
          "type": "boolean"
        },
        "preview_token": {
          "type": "string"
        },
        "backup_directory": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "restore": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "unstage": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "recreate_directories": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "staging_removed": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "backed_up": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "blocked": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "untracked": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "warnings": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "after": {
          "anyOf": [
            {
              "$ref": "#/$defs/Doctor"
            },
            {
              "type": "null"
            }
          ]
        }
      },
      "required": [
        "vault",
        "dry_run",
        "preview_token",
        "backup_directory",
        "restore",
        "unstage",
        "recreate_directories",
        "staging_removed",
        "backed_up",
        "blocked",
        "untracked",
        "warnings",
        "after"
      ],
      "additionalProperties": false
    },
    "Undo": {
      "type": "object",
      "properties": {
        "vault": {
          "type": "string"
        },
        "reverted_commit": {
          "type": "string"
        },
        "resulting_commit": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "dry_run": {
          "type": "boolean"
        },
        "finder_files_removed": {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "vault",
        "reverted_commit",
        "resulting_commit",
        "dry_run"
      ],
      "additionalProperties": false
    },
    "ErrorDetail": {
      "type": "object",
      "properties": {
        "code": {
          "type": "string"
        },
        "reason_code": {
          "type": "string"
        },
        "message": {
          "type": "string"
        }
      },
      "required": [
        "code",
        "reason_code",
        "message"
      ],
      "additionalProperties": false
    },
    "Error": {
      "type": "object",
      "properties": {
        "error": {
          "$ref": "#/$defs/ErrorDetail"
        }
      },
      "required": [
        "error"
      ],
      "additionalProperties": false
    },
    "Help": {
      "type": "object",
      "properties": {
        "help": {
          "type": "string"
        }
      },
      "required": [
        "help"
      ],
      "additionalProperties": false
    },
    "Version": {
      "type": "object",
      "properties": {
        "version": {
          "type": "string"
        }
      },
      "required": [
        "version"
      ],
      "additionalProperties": false
    },
    "Search": {
      "type": "object",
      "properties": {
        "query": {
          "type": "string"
        },
        "items": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/Skill"
          }
        },
        "warnings": {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "query",
        "items",
        "warnings"
      ],
      "additionalProperties": false
    },
    "ProjectSignal": {
      "type": "object",
      "properties": {
        "path": {
          "type": "string"
        },
        "category": {
          "type": "string"
        },
        "reason": {
          "type": "string"
        }
      },
      "required": [
        "path",
        "category",
        "reason"
      ],
      "additionalProperties": false
    },
    "StackSignal": {
      "type": "object",
      "properties": {
        "path": {
          "type": "string"
        },
        "stack": {
          "type": "string"
        },
        "reason": {
          "type": "string"
        }
      },
      "required": [
        "path",
        "stack",
        "reason"
      ],
      "additionalProperties": false
    },
    "SuggestMatch": {
      "type": "object",
      "properties": {
        "id": {
          "type": "string"
        },
        "matched_stacks": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "matched_category": {
          "type": "boolean"
        }
      },
      "required": [
        "id",
        "matched_stacks",
        "matched_category"
      ],
      "additionalProperties": false
    },
    "SuggestedBundle": {
      "type": "object",
      "properties": {
        "bundle": {
          "$ref": "#/$defs/Bundle"
        },
        "matched_categories": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "match_count": {
          "type": "integer"
        }
      },
      "required": [
        "bundle",
        "matched_categories",
        "match_count"
      ],
      "additionalProperties": false
    },
    "Suggest": {
      "type": "object",
      "properties": {
        "project": {
          "type": "string"
        },
        "categories": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "signals": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/ProjectSignal"
          }
        },
        "bundles": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/SuggestedBundle"
          }
        },
        "skills": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/Skill"
          }
        },
        "warnings": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "stacks": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "stack_signals": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/StackSignal"
          }
        },
        "matches": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/SuggestMatch"
          }
        }
      },
      "required": [
        "project",
        "categories",
        "signals",
        "bundles",
        "skills",
        "warnings",
        "stacks",
        "stack_signals",
        "matches"
      ],
      "additionalProperties": false
    },
    "RouterTarget": {
      "type": "object",
      "properties": {
        "agent": {
          "type": "string"
        },
        "path": {
          "type": "string"
        },
        "changed": {
          "type": "boolean"
        },
        "action": {
          "type": "string"
        },
        "previous_cli_path": {
          "type": "string"
        }
      },
      "required": [
        "agent",
        "path",
        "changed",
        "action"
      ],
      "additionalProperties": false
    },
    "BriefSkill": {
      "type": "object",
      "properties": {
        "id": {
          "type": "string"
        },
        "role": {
          "type": "string"
        },
        "when_to_use": {
          "type": "string"
        },
        "when_not": {
          "type": "string"
        },
        "description": {
          "type": "string"
        },
        "category": {
          "type": "string"
        },
        "stack": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "stage": {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "id",
        "role",
        "when_to_use",
        "when_not",
        "description",
        "category",
        "stack",
        "stage"
      ],
      "additionalProperties": false
    },
    "BriefBundleSkill": {
      "type": "object",
      "properties": {
        "id": {
          "type": "string"
        },
        "role": {
          "type": "string"
        }
      },
      "required": [
        "id",
        "role"
      ],
      "additionalProperties": false
    },
    "BriefBundle": {
      "type": "object",
      "properties": {
        "name": {
          "type": "string"
        },
        "members": {
          "type": "integer"
        },
        "title": {
          "type": "string"
        },
        "skills": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/BriefBundleSkill"
          }
        }
      },
      "required": [
        "name",
        "members"
      ],
      "additionalProperties": false
    },
    "BriefCatalog": {
      "type": "object",
      "properties": {
        "skills": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/BriefSkill"
          }
        },
        "bundles": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/BriefBundle"
          }
        },
        "bundles_total": {
          "type": "integer"
        },
        "total": {
          "type": "integer"
        },
        "limit": {
          "type": "integer"
        },
        "page": {
          "type": "integer"
        },
        "pages": {
          "type": "integer"
        },
        "hint": {
          "type": "string"
        }
      },
      "required": [
        "skills",
        "bundles",
        "bundles_total",
        "total",
        "limit",
        "page",
        "pages",
        "hint"
      ],
      "additionalProperties": false
    },
    "RouterInstall": {
      "type": "object",
      "properties": {
        "dry_run": {
          "type": "boolean"
        },
        "changed": {
          "type": "boolean"
        },
        "skill_hash": {
          "type": "string"
        },
        "cli_path": {
          "type": "string"
        },
        "targets": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/RouterTarget"
          }
        },
        "warnings": {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      },
      "required": [
        "dry_run",
        "changed",
        "skill_hash",
        "cli_path",
        "targets",
        "warnings"
      ],
      "additionalProperties": false
    },
    "SourceStatus": {
      "type": "object",
      "properties": {
        "id": {
          "type": "string"
        },
        "status": {
          "type": "string"
        },
        "reason_code": {
          "type": "string"
        },
        "repository": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "reference": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "tracking_kind": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "old_commit": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "new_commit": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "content_changed": {
          "anyOf": [
            {
              "type": "boolean"
            },
            {
              "type": "null"
            }
          ]
        },
        "error": {
          "anyOf": [
            {
              "$ref": "#/$defs/ErrorDetail"
            },
            {
              "type": "null"
            }
          ]
        }
      },
      "required": [
        "id",
        "status",
        "reason_code",
        "repository",
        "reference",
        "tracking_kind",
        "old_commit",
        "new_commit",
        "content_changed",
        "error"
      ],
      "additionalProperties": false
    },
    "Outdated": {
      "type": "object",
      "properties": {
        "items": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/SourceStatus"
          }
        },
        "groups_fetched": {
          "type": "integer"
        },
        "complete": {
          "type": "boolean"
        }
      },
      "required": [
        "items",
        "groups_fetched",
        "complete"
      ],
      "additionalProperties": false
    },
    "SourceFileInfo": {
      "type": "object",
      "properties": {
        "kind": {
          "type": "string"
        },
        "mode": {
          "type": "integer"
        },
        "bytes": {
          "type": "integer"
        },
        "content_hash": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        }
      },
      "required": [
        "kind",
        "mode",
        "bytes",
        "content_hash"
      ],
      "additionalProperties": false
    },
    "SourceFileDiff": {
      "type": "object",
      "properties": {
        "path": {
          "type": "string"
        },
        "change": {
          "type": "string"
        },
        "old": {
          "anyOf": [
            {
              "$ref": "#/$defs/SourceFileInfo"
            },
            {
              "type": "null"
            }
          ]
        },
        "new": {
          "anyOf": [
            {
              "$ref": "#/$defs/SourceFileInfo"
            },
            {
              "type": "null"
            }
          ]
        },
        "unified_diff": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "truncated": {
          "type": "boolean"
        }
      },
      "required": [
        "path",
        "change",
        "old",
        "new",
        "unified_diff",
        "truncated"
      ],
      "additionalProperties": false
    },
    "SourceUpdateWarning": {
      "type": "object",
      "properties": {
        "reason_code": {
          "type": "string"
        },
        "old_description": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "new_description": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        }
      },
      "required": [
        "reason_code",
        "old_description",
        "new_description"
      ],
      "additionalProperties": false
    },
    "Update": {
      "type": "object",
      "properties": {
        "id": {
          "type": "string"
        },
        "repository": {
          "type": "string"
        },
        "reference": {
          "type": "string"
        },
        "tracking_kind": {
          "type": "string"
        },
        "old_commit": {
          "type": "string"
        },
        "new_commit": {
          "type": "string"
        },
        "status": {
          "type": "string"
        },
        "old_content_hash": {
          "type": "string"
        },
        "new_content_hash": {
          "type": "string"
        },
        "files": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/SourceFileDiff"
          }
        },
        "old_audit": {
          "$ref": "#/$defs/Audit"
        },
        "new_audit": {
          "$ref": "#/$defs/Audit"
        },
        "warnings": {
          "type": "array",
          "items": {
            "$ref": "#/$defs/SourceUpdateWarning"
          }
        },
        "preview_token": {
          "type": "string"
        },
        "dry_run": {
          "type": "boolean"
        },
        "requires_confirmation": {
          "type": "boolean"
        },
        "changed": {
          "type": "boolean"
        },
        "committed": {
          "type": "boolean"
        },
        "diff_byte_limit": {
          "type": "integer"
        },
        "omitted_files": {
          "type": "integer"
        }
      },
      "required": [
        "id",
        "repository",
        "reference",
        "tracking_kind",
        "old_commit",
        "new_commit",
        "status",
        "old_content_hash",
        "new_content_hash",
        "files",
        "old_audit",
        "new_audit",
        "warnings",
        "preview_token",
        "dry_run",
        "requires_confirmation",
        "changed",
        "committed",
        "diff_byte_limit",
        "omitted_files"
      ],
      "additionalProperties": false
    },
    "Migration": {
      "type": "object",
      "additionalProperties": false,
      "required": [
        "version",
        "action",
        "source_state",
        "target_state",
        "journal",
        "preview_token",
        "changed",
        "dry_run",
        "projects",
        "warnings",
        "reason_codes",
        "routers",
        "state_files",
        "control_paths"
      ],
      "properties": {
        "version": {
          "type": "integer"
        },
        "action": {
          "type": "string"
        },
        "source_state": {
          "type": "string"
        },
        "target_state": {
          "type": "string"
        },
        "journal": {
          "anyOf": [
            {
              "type": "string"
            },
            {
              "type": "null"
            }
          ]
        },
        "preview_token": {
          "type": "string"
        },
        "changed": {
          "type": "boolean"
        },
        "dry_run": {
          "type": "boolean"
        },
        "projects": {
          "type": "array",
          "items": {
            "type": "object",
            "additionalProperties": false,
            "required": [
              "project",
              "from",
              "to",
              "state"
            ],
            "properties": {
              "project": {
                "type": "string"
              },
              "from": {
                "type": "string"
              },
              "to": {
                "type": "string"
              },
              "state": {
                "type": "string"
              }
            }
          }
        },
        "warnings": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "reason_codes": {
          "type": "array",
          "items": {
            "type": "string"
          }
        },
        "routers": {
          "type": "array",
          "items": {
            "type": "object",
            "additionalProperties": false,
            "required": [
              "agent",
              "from",
              "to",
              "state"
            ],
            "properties": {
              "agent": {
                "type": "string"
              },
              "from": {
                "type": "string"
              },
              "to": {
                "type": "string"
              },
              "state": {
                "type": "string"
              }
            }
          }
        },
        "state_files": {
          "type": "array",
          "items": {
            "type": "object",
            "additionalProperties": false,
            "required": [
              "from",
              "to",
              "kind",
              "operation",
              "mode"
            ],
            "properties": {
              "from": {
                "type": "string"
              },
              "to": {
                "anyOf": [
                  {
                    "type": "string"
                  },
                  {
                    "type": "null"
                  }
                ]
              },
              "kind": {
                "type": "string"
              },
              "operation": {
                "type": "string"
              },
              "mode": {
                "type": "integer"
              }
            }
          }
        },
        "control_paths": {
          "type": "array",
          "items": {
            "type": "string"
          }
        }
      }
    }
  }
}
```
<!-- cli-json-contract:end -->

إعادة التحقق من العقد وحده:

```sh
cargo test --locked -p kinana --test json_contract
```

تتعمد الاختبارات رفض المفتاح الزائد والمفقود والنوع الخاطئ داخل الكائنات المتداخلة وعناصر القوائم، وتقبل تغير المسارات والأوقات وترتيب المفاتيح. إضافة أمر جديد إلى clap بلا عقد/حالة اختبار تفشل فحص الجرد. لا تستدعي اختبارات العقد نموذجًا حيًا ولا تحتاج اتصالًا بالشبكة.

`search` يبحث بعبارات حرفية AND في الاسم والوصف النشط والدور والوسوم و«متى تُستخدم» (`when_to_use`)، حتى100 نتيجة مرتبة بملاءمة FTS5 (الاسم8، الوصف4، الدور2، الوسوم2، «متى تُستخدم»1) ثم id. البحث بوسم يعيد مهارته. حقل «متى لا تُستخدم» (`when_not`) لا يُبحث فيه ولا يُخزن في الفهرس. عند وجود أكثر من100 تطابق يذكر الاقتصاص في warnings؛100 بالضبط لا يولّد تحذيرًا. لا تُفسر معاملات FTS أو SQL من المستخدم. المدخل1–32 جزءًا حاويًا حروفًا/أرقامًا، حتى4096 بايت UTF-8، دون NUL. الفهرس المفقود (`INDEX_MISSING`) أو المنحرف عن ملفات المخزن (`INDEX_STALE`) يرجع خطأ مع إرشاد doctor --rebuild-index دون إعادة بناء ضمنية.

`suggest` يقرأ ملفات المشروع المحددة فقط؛ الإشارات تفسيرات قابلة للمراجعة وليست تصنيفًا بالنموذج. ترتب الحزم بعدد التصنيفات المختلفة المطابقة في مهارات primary، ثم الاسم؛ secondary لا يعطي تغطية افتراضية. بلا إشارات تُعرض جميع الحزم وقائمة skills فارغة؛ مع الإشارات تُعرض أيضًا المهارات المطابقة. manifest تالف/كبير/غير عادي أو رابط رمزي يُتخطى مع warning. يُعامل Cargo كإشارة backend-api، وPython project/build-system/Poetry كذلك؛ Flutter وExpo/React Native وios/android إلى mobile، وReact/Next إلى web-frontend، وDockerfile إلى devops. وجود React ضمن Expo وحده لا يضيف web-frontend. لا كتابة ولا تشغيل ملفات أو وكيل. إضافات الإصدار6 (التقنيات المكتشفة وترتيب المهارات وسبب كل تطابق) في قسم «أبعاد التصنيف» أدناه.

`init --install-router` يعيد Init مع `router: RouterInstall` حاضرًا؛ init العادي يحذفه. `changed` يصف الحاجة للتثبيت أو الترقية حتى في dry-run، وtargets الوجهتان المحددتان دائمًا، ولكل منهما `action` (إصدار7): `install` أو `upgrade` أو `unchanged`. التثبيت المطابق لا يغيّر شيئًا. مجلد أجنبي أو مملوك معدّل/مستبدل يرجع ROUTER_CONFLICT ورمز2؛ لا overwrite لغير النسخة المملوكة غير المعدلة من نص سابق، ولا إعدادات عامة أخرى. عند الفشل يحفظ ما عدّله المستخدم ويذكر مسار الاستعادة في رسالة الخطأ.

## ترحيل Kinana — عقد إصدار4

`migrate --dry-run --json` يعرض الخطة بلا أي كتابة. الوضع الافتراضي بلا --dry-run أو --confirm يعرض Migration ويرجع2 عند وجود تغيير، و0 إذا لا يوجد تغيير. --confirm يرتبط ببصمة المعاينة، ويعيد التقرير نفسه بعد التنفيذ مع dry_run=false وjournal لمسار الاستعادة. --rollback يستخدم الشكل نفسه مع action=rollback. الحقول والمفاتيح ثابتة، ولا يضيف نقل ملف القفل حقولًا إلى أشكال use/status الحالية.

تقرأ الأوامر الحالة والقفل القديمين دون كتابة. أوامر التعديل قبل الترحيل ترفض برمز2 وMIGRATION_REQUIRED. حالات التعارض/تغير البصمة/المصدر/المقاطعة/انشغال الكاتب ترفض برمز2 وMIGRATION_CONFLICT أو MIGRATION_PREVIEW_CHANGED أو MIGRATION_CHANGED أو MIGRATION_SOURCE_CHANGED أو MIGRATION_INCOMPLETE أو STATE_CONFLICT أو STATE_BUSY. أخطاء IO/SQLite التنفيذية تبقى برمز1 وفق عقد Error. NO_MIGRATION قرار2 عند طلب تراجع غير موجود. لا كتابة ضمن القراءة أو المعاينة، ولا حقل needs_decision في JSON.

يتضمن Migration أيضًا routers للوجهتين العامتين المملوكتين، وstate_files لكل عنصر مع العملية والصلاحية والوجهة (null لـWAL/SHM المحفوظين في الأصل والاستعادة)، وcontrol_paths لكل مسارات التحكم والاستعادة. في المعاينة journal هو جذر الاستعادة؛ بعد التنفيذ هو مجلد العملية المحدد. التراجع يحتفظ بالحالة الجديدة في rolled-back-state، ولا يحذف بياناتها.

## حذف مهارة أو حزمة — عقد إصدار5

إشارات أقسام الإصدارات 5 إلى 8 إلى SPEC.md تعني النص بعد تطبيق phase-8-spec-proposal.diff، وهو مقترح لم يعتمده المالك بعد ولم يُطبَّق على SPEC.md.

الإصدار5 يضيف عقدي `remove` و`bundle delete` صراحةً وفق §5.7 من SPEC.md، دون تغيير أي شكل قائم. هما الأمران الوحيدان اللذان يحذفان من المخزن، ولا يكتبان في أي مشروع أو ملف قفل.

`remove <id>` يعيد Remove. `removed_paths` مسارات نسبية إلى جذر المخزن بفاصل `/` لما حُذف أو سيُحذف، بالترتيب: `skills/<id>` ثم `meta/<id>.toml` ثم `dist/<id>` ثم `dist/.manual/<id>`؛ يُذكر مسارا dist فقط إذا وُجدت النسخة المولدة. `bundles_changed` أسماء الحزم التي أُزيل منها عنصر المهارة بترتيب الاسم، و`[]` إن لم تذكرها حزمة. المسارات للعرض، ولا تعطي عميل CLI إذن حذف.

`bundle delete <name>` يعيد BundleDelete، ويحذف `bundles/<name>.toml` وحده.

في الأمرين، كما في `pin`: مع `--dry-run` يكون `dry_run=true` و`changed=true` و`committed=false` دون أي كتابة (لا ملفات ولا تجهيز ولا Git ولا فهرس)؛ والتنفيذ الناجح `changed=true` و`committed=true` بـcommit واحد يُتراجع عنه بـ`undo`. لا توجد نتيجة ناجحة بـ`changed=false`: الهدف غير الموجود خطأ.

| الحالة | `code` | رمز الخروج |
| --- | --- | --- |
| معرّف مهارة غير آمن (فاصل مسار، `..`، مسار مطلق، أحرف غير مسموحة) | `INVALID_SKILL_ID` | 1 |
| مهارة غير موجودة في المخزن | `SKILL_NOT_FOUND` | 1 |
| رابط رمزي في مسار المخزن أو مكان مجلد المهارة أو meta أو ملف الحزمة أو dist | `SYMLINK_PATH` | 1 |
| المهارة مفعّلة في مشروع مسجل؛ الرسالة تذكر مسارات المشاريع، والحل `unuse` أولًا | `SKILL_IN_USE` | 2 |
| الحذف يترك حزمة مرفوضة بقواعد التحقق الحالية؛ الرسالة تذكر اسم الحزمة | `BUNDLE_WOULD_BE_INVALID` | 2 |
| في مجلد المهارة ما لا يعيده `undo` (مجلد `.git` متداخل، ملف لا يتتبعه Git، مجلد فارغ غير مسجل)؛ الرسالة تذكر المسارات (إصدار8) | `SKILL_HAS_UNRECOVERABLE_CONTENT` | 2 |
| النسخة المولدة في dist معدلة أو غير مملوكة | `BUILD_CONFLICT` | 2 |
| اسم حزمة غير آمن | `INVALID_BUNDLE_NAME` | 1 |
| حزمة غير موجودة | `BUNDLE_NOT_FOUND` | 1 |
| مخزن غير نظيف أو حالة تغيرت أثناء الفحص | `GIT_DIRTY_VAULT` / `BUNDLE_CHANGED` / `ACTIVATION_CONFLICT` | 2 |
| سُجّل الـcommit ثم تعذر تحديث الفهرس | `INDEX_REFRESH_FAILED` | 1 |
| سُجّل الـcommit ثم تعذر حذف مجلد التجهيز الخاص | `REMOVAL_CLEANUP_FAILED` | 1 |

قفل مشروع مسجل تتعذر قراءته أو التحقق منه يوقف `remove` بخطئه الأصلي (مثل `INVALID_PROJECT_LOCK` أو `INVALID_TOML` أو `SYMLINK_PATH`) مع مسار المشروع في الرسالة، لأن عدم الاستخدام غير مثبت. مشروع مسجل مجلده مفقود أو بلا قفل أو قفله لمخزن آخر لا يمنع الحذف. الحزمة التي تبقى بلا مهارات مقبولة في قواعد التحقق الحالية، فتُحفظ بقائمة `skills` فارغة وتظهر في `bundles_changed`.

في الحالتين الأخيرتين من الجدول الحذف مسجّل في Git رغم رمز الخروج1؛ الرسالة تذكر ذلك. أعد قراءة `list` أو `bundle list` بدل افتراض بقاء الهدف، والإصلاح `doctor --rebuild-index` أو إزالة مجلد التجهيز المذكور. `show` و`pin` يعيدان لمعرّف غير موجود `IO_ERROR` مع `reason_code: IO_NOT_FOUND` كما كانا؛ `remove` يستعمل `SKILL_NOT_FOUND` المستخدم في `classify` و`use`.

## أبعاد التصنيف — عقد إصدار6

الإصدار6 يضيف بُعدي «التقنية» (`stack`) و«مرحلة العمل» (`stage`) وفق §3 «أبعاد التصنيف» و§5.3 و§6 من SPEC.md. كل التغييرات إضافات: لا يُحذف مفتاح قائم ولا يتغير نوعه. `category` يبقى «المجال» بقيمة واحدة.

**حقول جديدة حاضرة دائمًا في `Skill`** (أي في `list` و`search.items` و`show.skill` و`suggest.skills`):

| المفتاح | النوع | المعنى |
| --- | --- | --- |
| `stack` | `string[]` | قيم من قائمة `stacks` في box.toml بترتيب القائمة؛ `[]` لمهارة بلا تقنية أو ملف meta قديم |
| `stage` | `string[]` | قيم من قائمة `stages` بترتيب القائمة؛ `[]` عند الغياب |
| `when_to_use` | `string` | نص «متى تُستخدم» من التصنيف، وهو مما يُبحث فيه؛ نص فارغ عند الغياب |

`Metadata.classification` و`Metadata.proposal` يحملان `stack` و`stage` فقط إذا كانا غير فارغين (انظر قاعدة الحذف أعلاه). `ClassifiedItem` لا يتغير؛ اقرأ البُعدين بعد التصنيف من `show` أو `list`.

**`dimensions <id> [--stack <value>]... [--stage <value>]... [--clear-stack] [--clear-stage]`** يعيد Dimensions: `id: string` و`stack: string[]` و`stage: string[]` (القيم بعد التعديل أو في معاينته، بترتيب القائمة ودون تكرار) و`dry_run: boolean` و`changed: boolean` و`committed: boolean`. الخيار المذكور يستبدل قائمة بُعده كاملة، وتكراره يضيف قيمة؛ `--clear-*` يفرغ البُعد ولا يجتمع مع خياره؛ البُعد غير المذكور يبقى كما هو. كما في `pin`: المعاينة `changed` تصف الفرق المتوقع و`committed=false` دون أي كتابة؛ الطلب المطابق للحالة الحالية `changed=false` و`committed=false` بلا commit؛ التنفيذ الفعلي commit واحد يُتراجع عنه بـ`undo`. لا يغيّر `classified_by` ولا الوصف ولا الاقتراح المنتظر ولا dist.

**`suggest`** يضيف ثلاثة مفاتيح حاضرة دائمًا:

| المفتاح | النوع | المعنى |
| --- | --- | --- |
| `stacks` | `string[]` | قيم `stack` المكتشفة من ملفات المشروع، بترتيب قائمة box.toml، ومنها فقط |
| `stack_signals` | `StackSignal[]` | لكل إشارة: `path: string` و`stack: string` و`reason: string` |
| `matches` | `SuggestMatch[]` | بترتيب `skills` نفسه وعدده: `id: string` و`matched_stacks: string[]` و`matched_category: boolean` |

الإشارات: `package.json` صالح إلى `node`، واعتماداته `react`/`next`/`expo`/`react-native` إلى القيمة بالاسم نفسه، و`pubspec.yaml` لـFlutter إلى `flutter`، و`Cargo.toml` إلى `rust`، و`pyproject.toml` إلى `python`، و`Dockerfile` إلى `docker`، و`Package.swift` أو مجلد `*.xcodeproj` / `*.xcworkspace` في جذر المشروع إلى `swift`. `signals` و`categories` لا يتغير معناهما. تُعرض المهارة إذا طابقت تقنية مكتشفة أو مجالًا مكتشفًا، بالترتيب: عدد التقنيات المطابقة تنازليًا، ثم المطابِقة للمجال أيضًا، ثم id؛ فمهارات React تسبق بقية `web-frontend` في مشروع React. ترتيب `bundles` كما كان. رابط رمزي مكان علامة Xcode يُتخطى مع warning.

**رموز الأخطاء** (كلها `Error` بالشكل المعتاد):

| الحالة | `code` | رمز الخروج |
| --- | --- | --- |
| قيمة `stack` ليست في قائمة `stacks`: في `dimensions`، في `list --stack`، في أي كتابة لـmetadata تغيّر البُعد، أو في اقتراح محفوظ عند `review --accept` أو إعادة كتابته | `INVALID_STACK` | 1 |
| قيمة `stage` ليست في قائمة `stages`، في المواضع نفسها (`list --stage`) | `INVALID_STAGE` | 1 |
| `dimensions` بلا أي خيار، أو خيار مع `--clear-*` لبُعده | `INVALID_ARGUMENTS` | 1 |
| قائمة `stacks` أو `stages` في box.toml أو في طلب الإعدادات فيها عنصر فارغ أو بمسافات طرفية أو محرف تحكم أو مكرر | `INVALID_STACKS` / `INVALID_STAGES` | 1 |
| حفظ إعدادات يحذف من القائمة قيمة ما زالت تحملها مهارة أو اقتراح محفوظ؛ الرسالة تذكر القيم ومعرّفات المهارات | `STACK_IN_USE` / `STAGE_IN_USE` | 1 |
| صفوف المخزن في الفهرس تختلف عن ملفاته، ومنها نص «متى تُستخدم»، عند `search` و`list --query`؛ لا إصلاح ضمني | `INDEX_STALE` | 1 |
| ملف فهرس يحمل نسخة1 أو2 لكن أعمدة جداوله لا تطابق تخطيط نسخته كاملة، أو كتابة في ملف نسخة1 (فهرس بيتا 0.1.0)؛ لا يُعدَّل | `INDEX_CONFLICT` | 2 |

`INVALID_STACKS` و`STACK_IN_USE` ونظيراهما تصدر من مسار إعدادات core الذي تستعمله الواجهة (لا يوجد أمر CLI لتعديل القائمتين بعد)، و`INVALID_STACKS` / `INVALID_STAGES` أيضًا من `classify` إذا عُدّل box.toml يدويًا بقائمة غير صالحة. قيمة خارج القائمة في رد الوكيل ليست خطأ CLI: يُرفض الرد كله كتصنيف خارج قائمته (`INVALID_AGENT_PROPOSAL` داخل تحذيرات العنصر)، ثم محاولة واحدة، ثم الرجوع للكلمات المفتاحية كما في بقية أخطاء المخطط.

**التصنيف:** مخطط رد الوكيل صار عشرة حقول إلزامية بإضافة `stack` و`stage` مقيدين بالقائمتين، و`Classify.estimated_tokens.fixed_tokens_per_call` يكبر تبعًا لذلك (نص التعليمات والقائمتان والمخطط داخل الطلب، ونسخة المخطط الثانية عبر `--json-schema`)؛ شكل `Estimate` و`Calibration` لا يتغير. البديل بالكلمات يضيف إلى `stack` التقنيات المسماة صراحة في الاسم والوصف الأصلي فقط، ولا يحذف قيمة موجودة ولا يكتب `stage`.

**الفهرس:** نسخة مخططه صارت2 بإضافة عمود `when_to_use`، وتُكتب في ملف مستقل `~/.kinana/index-v2.db` يُبنى من ملفات المخزن. فهرس بيتا 0.1.0 (`~/.kinana/index.db`، نسخة1) لا يُرحَّل ولا يُكتب، والبيتا يستمر في استعماله. التفاصيل في قسم «عقد إصدار8» أدناه وفي [migration.md](migration.md).

## الفهرس المختصر ونص الموجّه — عقد إصدار7

الإصدار7 يضيف `list --brief` ويضيف مفتاحًا واحدًا إلى `RouterTarget`، وفق §4 و§6 من SPEC.md. لا يُحذف مفتاح قائم ولا يتغير نوعه. الإصدار8 يضيف مفاتيح الصفحات إلى `BriefCatalog` (القسم التالي).

**`list --brief [--limit <n>] [--page <n>]`** يعيد `BriefCatalog`، كائنًا في الجذر بثمانية مفاتيح حاضرة دائمًا:

| المفتاح | النوع | المعنى |
| --- | --- | --- |
| `skills` | `BriefSkill[]` | صفحة واحدة من المهارات بعد المرشحات، بترتيب `list` نفسه (المفضلة أولًا ثم id) |
| `bundles` | `BriefBundle[]` | الحزم بترتيب الاسم في الصفحة الأولى، بأعضائها كاملين ما دامت تتسع في ميزانية الصفحة (أدناه)؛ `[]` في الصفحات التالية |
| `bundles_total` | `integer` | عدد كل حزم المخزن؛ أكبر من طول `bundles` في الصفحات التالية وحين لا تتسع الصفحة لها كلها |
| `total` | `integer` | عدد المهارات التي تجتاز المرشحات في كل الصفحات |
| `limit` | `integer` | حجم الصفحة: 20 افتراضيًا، أو قيمة `--limit` من 1 إلى 50 |
| `page` | `integer` | رقم هذه الصفحة، ابتداءً من 1 |
| `pages` | `integer` | عدد الصفحات، 1 على الأقل |
| `hint` | `string` | رسالة تطلب التضييق وتذكر الصفحة التالية إن وُجدت، وتذكر إن قُص نص الصفحة أقصر من 200 حرف، وإن عُرضت الحزم بالاسم وعدد الأعضاء فقط (مع `kinana bundle show <name>`)، وإن لم تُعرض كل الحزم (مع `kinana bundle list`)؛ فارغة حين تحمل الصفحة كل ما يجتاز المرشحات وكل الحزم كاملة ولم يُقص نصها أقصر |

`BriefSkill`، وكل مفاتيحه حاضرة دائمًا:

| المفتاح | النوع | المعنى |
| --- | --- | --- |
| `id` | `string` | معرّف المهارة، وهو ما يُمرَّر إلى `use` |
| `role` | `string` | الدور من التصنيف؛ فارغ لمهارة لم تُصنَّف، ونسخة من الوصف الأصلي للمصنفة بالكلمات المفتاحية. مقصوص مثل `description` |
| `when_to_use` | `string` | «متى تُستخدم»؛ فارغ لغير المصنفة بالوكيل. مقصوص مثل `description` |
| `when_not` | `string` | «متى لا تُستخدم»، من ملف meta؛ فارغ لغير المصنفة بالوكيل. مقصوص مثل `description` |
| `description` | `string` | الوصف النشط مقصوصًا إلى 200 حرف على الأكثر على حد حرف، وآخره `…` إذا قُص؛ أو إلى 120 ثم 60 حرفًا إذا تجاوزت الصفحة ميزانيتها (أدناه) |
| `category` | `string` | المجال |
| `stack` | `string[]` | قيم التقنية بترتيب قائمة box.toml؛ `[]` عند الغياب |
| `stage` | `string[]` | قيم مرحلة العمل بترتيب القائمة؛ `[]` عند الغياب |

`BriefBundle`: `name: string` و`members: integer` و`title: string` و`skills: BriefBundleSkill[]`. `name` و`members` (عدد أعضاء الحزمة) حاضران دائمًا. `title` (مقصوص إلى 200 حرف) و`skills` حاضران إلا حين تعرض الصفحة الحزم بالاسم وعدد الأعضاء فقط، فيغيبان معًا، و`BriefBundleSkill`: `id: string` و`role: string` (`primary` أو `secondary`، وهي أولوية العضو في الحزمة لا دور المهارة). `agents` و`group` غير معروضين؛ `bundle show` و`suggest` يعرضانهما.

- يقبل مرشحات `list` كلها (`--stack` و`--stage` و`--category` و`--agent` و`--audit` و`--pinned` و`--state` و`--query`) مجتمعة، كلها مطلوبة معًا، بالتحقق ورموز الأخطاء نفسها: `INVALID_STACK` و`INVALID_STAGE` و`INVALID_AGENT` برمز1، وأخطاء البحث مع `--query`. قيمة صحيحة لا تحملها أي مهارة تعيد `skills: []`.
- **لا يُعاد الفهرس كاملًا في رد واحد.** الحد في الكود ويسري مع المرشحات ودونها. صفحة بعد الأخيرة ليست خطأ: `skills: []` و`hint` يذكر عدد الصفحات. `--limit` خارج 1–50، أو `--page` أقل من 1، أو أي منهما دون `--brief`: `INVALID_ARGUMENTS` برمز1.
- المرشحات تخص `skills` وحدها. `bundles` لا تُصفّى ولا تُقسَّم صفحات: الحزمة هي ما يفعّله `use <bundle>` كاملًا.
- يُقرأ من ملفات المخزن: يعمل والفهرس مفقود، ولا يكتب فهرسًا ولا أي ملف. `--query` وحده يمر بالبحث وفهرسه كما في `list`، وتحذيراته تُكتب إلى stderr.
- **حد طول النصوص وميزانية الصفحة:** `role` و`when_to_use` و`when_not` و`description` و`title` الحزمة تُقص إلى 200 حرف. مصفوفتا `skills` و`bundles` معًا لا تتجاوزان 1,100 بايت من JSON المضغوط لكل مهارة يسمح بها `limit`. إذا لم تتسع الحزم كاملة بجوار المهارات بنصها الكامل، تُعرض بالاسم وعدد الأعضاء فقط ويُضاف ذلك إلى `hint` مع `kinana bundle show <name>`. ثم إذا لم تتسع المهارات بجوارها، والحزم محسوبة بربع الميزانية على الأكثر، تُقص نصوص المهارات كلها إلى 120 حرفًا ثم إلى 60، ويُضاف إلى `hint` أن النص قُص وأن `kinana show <id>` يعرضه كاملًا. وأخيرًا تُعرض من الحزم بترتيب الاسم ما يتسع في الباقي، ويذكر `hint` كم عُرض من `bundles_total` وأن `kinana bundle list` يعرضها كلها. مهارات الصفحة وترتيبها وعدد الصفحات لا تتغير. `category` يُقص كذلك، و`stack` و`stage` لا يحملان إلا قيم قائمتيهما في `box.toml`، كل قيمة مرة واحدة بترتيب القائمة. المعرّف واسم الحزمة لا يُقصان ولا حد لطولهما، وكذلك قيم `stack` و`stage` المسموحة نفسها في `box.toml` (لا حد لعددها ولا لطولها، والمكرر منها يُعرض مرة واحدة)؛ فهذه وحدها قد تتجاوز الميزانية: صفحة معرّفاتها أو قيمها المسموحة طويلة جدًا تتجاوز الحد بقدر ما زادت عليه، والمحرك لا يحذف مهارة من الصفحة لذلك. لا يُحوَّل حقل بغير القص وحذف محارف التحكم (البند التالي). الحرف هنا Unicode scalar، فالقص لا يشطر حرفًا لكنه قد يفصل علامة تشكيل عن حرفها.
- **كل النصوص بيانات** كتبها مؤلفو المهارات والحزم أو المصنّف، وليست تعليمات للعميل. تُحذف من كل نص في JSON محارف التحكم (فئة Unicode `Cc`، ومنها C1) وتسلسلات الطرفية كاملة: CSI (`ESC [` إلى حرفه الأخير) وOSC (`ESC ]` إلى BEL أو `ESC \`) والتسلسل ذو الحرفين، ويصير فاصل السطر (LF وCR وVT وFF وNEL) وTab مسافة. محارف U+2028 وU+2029 ومحارف الاتجاه ليست محارف تحكم فتبقى في JSON. الناتج النصي بلا `--json` سطر `Skills: N of T (page P of Q)` ثم سطر لكل مهارة (`id` ثم المجال ثم `stack=` و`stage=` و`role=` و`use=` و`not=` و`desc=` يفصلها Tab)، ثم `Bundles: M of N` وسطر لكل حزمة (`name` ثم `title` ثم الأعضاء، أو `name` ثم `members=<عدد>` في العرض المختصر)، ثم سطر `Hint: ...` إن وُجدت رسالة؛ تُستبدل فيه بمسافة محارف التحكم وU+2028 وU+2029 ومحارف فرض الاتجاه U+202A–U+202E وU+2066–U+2069. شكل الناتج النصي للعرض وليس عقدًا.

الحجم المقاس على مخزن اختبار من 200 مهارة مصنفة بالوكيل و8 حزم، بتقدير `ceil(bytes / 4)` للتوكنات: قبل حد الصفحة كان الرد بلا مرشح 178,518 بايتًا (نحو 44,630 توكنًا)؛ بعده وبعد حد طول النصوص الصفحة الافتراضية 20,912 بايتًا (نحو 5,228 توكنًا)، وأكبر صفحة ممكنة (`--limit 50`) 45,146 بايتًا (نحو 11,287 توكنًا)، و`--stack react` لاثنتي عشرة مهارة 14,507 بايتًا (نحو 3,627 توكنًا). بحقول من نحو 30,000 حرف في كل مهارة تبقى الصفحة الافتراضية 13,674 بايتًا (نحو 3,419 توكنًا) وأكبر صفحة 33,814 بايتًا (نحو 8,454 توكنًا). ومع 300 حزمة بأسماء طويلة وستين عضوًا في كل منها، الصفحة الافتراضية نحو 5,618 توكنًا وفيها 29 حزمة بالاسم والعدد، وأكبر صفحة نحو 13,826 توكنًا وفيها 72. التفصيل وشروط القياس في §4 من المقترح phase-8-spec-proposal.diff. ضيّق بالمرشحات قبل طلب صفحة تالية.

**`init --install-router`:** `RouterTarget` يضيف `action: string` حاضرًا دائمًا:

| `action` | حالة الوجهة | `changed` |
| --- | --- | --- |
| `install` | لا شيء في المسار | `true` |
| `upgrade` | نسخة مملوكة غير معدلة بنص شُحن سابقًا؛ تُستبدل بالنص الحالي بتبديل ذري | `true` |
| `relocate` | نسخة مملوكة غير معدلة بالقالب الحالي تسمي مسار CLI آخر (انتقل التطبيق)؛ يُكتب المسار الحالي بتبديل ذري. `previous_cli_path` يحمل المسار القديم، ويغيب في غير هذا الإجراء | `true` |
| `unchanged` | نسخة مملوكة غير معدلة بالنص الحالي | `false` |

`--dry-run` يعرض الإجراء نفسه ولا يكتب شيئًا. `RouterInstall.changed` صحيح إذا تغيرت وجهة واحدة على الأقل، و`skill_hash` تجزئة النص المثبت (القالب بالمسار) دائمًا. **مسار الـCLI (قرار المالك، 2026-11-17):** `RouterInstall.cli_path` المسار الكامل الذي يسميه النص المثبت: الملف التنفيذي `kinana` الذي يعمل، أو `kinana` بجوار البرنامج الذي يعمل (`Kinana.app/Contents/MacOS/kinana` بجوار التطبيق)، بعد حل الروابط. يُكتب في النص كلمة صدفة واحدة بين علامتي اقتباس مفردتين وكل `'` فيه `'\''`. يُرفض بـ`ROUTER_CLI_PATH_INVALID` ورمز2 دون كتابة إذا لم يكن مطلقًا أو UTF-8، أو فيه محرف تحكم أو فاصل سطر أو `` ` ``، أو تجاوز مقتبسًا 400 بايت، أو تجاوز النص المثبت 3,584 بايتًا؛ وبـ`ROUTER_CLI_PATH_TEMPORARY` ورمز2 إذا كان التطبيق في جذر صورة قرص مركبة (`/Volumes/<صورة>/<اسم>.app`) أو في App Translocation. تطبيق على قرص خارجي (`/Volumes/<قرص>/Applications/...`) مقبول. ملف الملكية للنص ذي المسار إصداره 2 ويضيف `cli_path` و`template_hash`؛ بيتا 0.1.0 تعدّه تعارضًا ولا تكتب فوقه (اختبار بالملف التنفيذي الحقيقي). بعد ترقية ناجحة قد يحمل `warnings` مسار نسخة سابقة تغيرت أثناء العملية فحُفظت. نسخة مملوكة عُدّلت، أو النص السابق بلا ملف ملكية، أو مجلد أجنبي، أو رابط رمزي مكان الوجهة، يبقى `ROUTER_CONFLICT` ورمز2 دون أي كتابة في الوجهتين. مسار إعدادات الواجهة يعرض الترقية بالسبب `UPGRADE_ROUTER` في خطته (ليس ناتج CLI)، ورمز تأكيده يغطي الإجراء المعروض لكل وجهة.

## قرارات مراجعة المالك — عقد إصدار8

الإصدار8 يضيف ستة مفاتيح إلى `BriefCatalog` (`bundles_total` و`total` و`limit` و`page` و`pages` و`hint`، في القسم السابق) ومفتاح `members` إلى `BriefBundle`، ويغيّر سلوكيات دون تغيير شكل أي ناتج آخر. الاستثناء الوحيد من قاعدة عدم حذف المفاتيح: `title` و`skills` في `BriefBundle` يغيبان حين تعرض الصفحة الحزم بالاسم وعدد الأعضاء فقط. الإصدار7 الذي أضافهما لم يُشحن في أي إصدار (البيتا 0.1.0 على الإصدار4)، فلا عميل منشور يعتمد على حضورهما. لا يتغير نوع أي مفتاح. رمز الخطأ الجديد الوحيد `SKILL_HAS_UNRECOVERABLE_CONTENT` (أدناه)؛ `INDEX_MIGRATION_FAILED` الذي وُثّق في إصدار6 لم يعد يصدر لأن الترحيل في المكان أُلغي.

**ملف الفهرس:** قيمة `index` في `Init` و`Doctor` و`Rebuild` صارت مسار `~/.kinana/index-v2.db`. بعد الترقية من بيتا 0.1.0 يبني أول أمر ليس معاينة هذا الملف من ملفات المخزن، ولا يغيّر `~/.kinana/index.db` (فهرس البيتا) بايتًا واحدًا. ما يراه العميل:

| الحالة | الناتج |
| --- | --- |
| أول أمر بلا `--dry-run` بعد الترقية | ينجح كالمعتاد؛ الفهرس يُبنى قبله دون ذكر في ناتجه |
| أمر مع `--dry-run` يحتاج الفهرس قبل بنائه (`search` و`list --query` و`use` و`remove` وغيرها) | `INDEX_MISSING` برمز1، بلا أي كتابة |
| `search` أو `list --query` بعد أن غيّر البيتا المخزن | `INDEX_STALE` برمز1 إلى `doctor --rebuild-index` أو أي أمر يغيّر المخزن؛ البحث لا يكتب الفهرس |
| ملف أجنبي أو مجهول في `index-v2.db` | `INDEX_CONFLICT` برمز2؛ لا يُعدَّل ولا يُبنى فوقه |
| كتابة في ملف نسخة1، ومنها `doctor --rebuild-index` | `INDEX_CONFLICT` برمز2؛ لا يُعدَّل |

في الحالة القديمة `~/.skillsbox` (للقراءة فقط قبل `migrate`) يبقى مسار الفهرس `index.db`، والبحث يجيب من أعمدته الموجودة دون «متى تُستخدم» ولا يكتب شيئًا.

**`undo`:** يعيد `SKILL_IN_USE` برمز2 إذا كان التراجع عن آخر commit يزيل مهارة مفعّلة في مشروع مسجل لهذا المخزن، في التنفيذ وفي `--dry-run` وقبل أي كتابة. الرسالة تذكر المهارة والمشاريع وأن `unuse` لكل مشروع يسبق التراجع. شكل `Undo` لا يتغير إلا بالحقل الاختياري أدناه.

**`build` و`undo` وملفات Finder (قرار المالك 2026-11-17):** قبل فحص التجزئة يبحث الأمران عن ملفات `.DS_Store` لا يتتبعها Git داخل `skills/<id>/` في أي عمق. إذا كان حذفها يجعل تجزئة مجلد المهارة تساوي `content_hash` المسجل تمامًا، فهي الفرق الوحيد: تُحذف من القرص ويذكرها الحقل الاختياري `finder_files_removed` في `Build` و`Undo` بمسارات نسبية للمخزن، ويذكرها الناتج النصي في سطر أخير بصيغة الهروب الموحّدة (أدناه في `doctor`)؛ الحقل في JSON يحمل المسارات كما هي. في `--dry-run` يذكر الحقل ما سيُحذف ولا يُحذف شيء، والتحقق يجري كأنها حُذفت. لا يُحذف شيء إلا بعد أن يثبت أن الأمر كله صالح بدونها؛ رفض `undo` مثل `UNDO_UNMANAGED_COMMIT` أو `SKILL_IN_USE` يتركها مكانها. أي فرق آخر في المهارة يبقي `SOURCE_HASH_MISMATCH` ولا يحذف شيئًا. `.DS_Store` المتتبع جزء من المهارة المستوردة ولا يُحذف. الحذف ليس commit لأن الملفات غير متتبعة، فلا يعيدها `undo`. تعريف التجزئة لم يتغير، فيحسب البيتا 0.1.0 التجزئة نفسها، والأوامر الأخرى ترفض كما كانت ولا تحذف شيئًا. الحقل غائب حين لا يوجد شيء، و`build` الداخلي في الأوامر الأخرى لا يضيفه. **`use` (قرار المالك، الجولة السادسة):** إذا كانت ملفات `.DS_Store` غير المتتبعة هي الفرق الوحيد في المهارات المطلوبة، تذكرها رسالة `SOURCE_HASH_MISMATCH` بمساراتها، بصيغة الهروب الموحّدة، وتقترح `kinana build` بمعرّفات تلك المهارات وحدها؛ مع أي فرق آخر تبقى الرسالة كما كانت بلا اقتراح. الرمز ورمز الخروج لا يتغيران، و`use` لا يكتب شيئًا.

**`remove` ومحتوى لا يعيده `undo`:** مجلد `.git` في أي عمق داخل `skills/<id>/`، أو ملف لا يتتبعه Git، أو مجلد فارغ لا يسجله ملف meta، يوقف `remove` بـ`SKILL_HAS_UNRECOVERABLE_CONTENT` برمز2، في التنفيذ وفي `--dry-run` وقبل أي كتابة وقبل فحص نظافة المخزن. الرسالة تذكر حتى عشرة مسارات. `.DS_Store` غير المتتبع داخل مجلد المهارة ليس محتوى لهذا الفحص، ولا يجعل المخزن غير نظيف، ويُحذف مع المهارة (قرار المالك 2026-11-17). شكل `Remove` لا يتغير.

**`doctor` والتغيير المنقطع:** `checks` يضم فحصًا جديدًا `name: "interrupted_change"` و`reason_code: "DOCTOR_INTERRUPTED_CHANGE"`. يفشل إذا خالفت ملفات المخزن آخر commit، أو بقيت مجلدات تجهيز باسم يبدأ بـ`.skillsbox-` في جذر المخزن أو `dist/` أو `.git/`، أو لم تتطابق مهارة في `skills/` و`meta/`، أو بقيت نسخة مولدة في `dist/` بلا مهارتها؛ مهارة بلا نسخة مولدة (مخزن لم يُبنَ بعد) لا تُفشله. رسالته تذكر ما وُجد فقط، ولا تطبع أي أمر شل ولا مسار المخزن (قرار المالك، الجولة السادسة): الملفات التي تخالف آخر commit، والمجلدات الفارغة الناقصة التي يسجلها ملف meta فيه، والمسارات التي حل محلها شيء آخر وتطلب نقله أولًا، والملفات التي ليست في آخر commit وتبقى مكانها، ومجلدات التجهيز. تقترح `kinana doctor --repair` لمعاينة إعادة الملفات المخالفة وحدها، لا المخزن كله، ثم `kinana build`. كل اسم بصيغة هروب واحدة: `\` يُكتب `\\`، وكل محرف يمكن أن يُظهر اسمًا كاسم آخر أو يغيّر سطره يُكتب `\u{..}`: محارف التحكم، ومحارف الاتجاه، وفاصلا السطر والفقرة، والمحارف الخفية والتنسيقية (الشرطة اللينة، والمحارف صفرية العرض ومنها الوصل وعدم الوصل، ومانع الكسر، ومحددات الأشكال، والوسوم، وحشوات الهانغل)، وكل مسافة غير مسافة ASCII العادية. الحروف المتشابهة شكلًا من كتابات أخرى لا تُهرَّب. القائمة تقف عند عشرة وتذكر عدد الباقي (§5.7 من SPEC.md)، وفشله يجعل `healthy: false` ورمز الخروج1 كأي فحص. شكل `Doctor` لا يتغير.

**`classify` المتوقف (قرار المالك، الجولة الثامنة):** حقل اختياري `stopped` في `Classify`، ضمن النسخة 8، يظهر حين يتوقف التشغيل قبل آخر مهارة: `reason_code` هو `CANCELLED` (إلغاء من التطبيق) أو `AGENT_USAGE_LIMIT` (حد استخدام Claude Code) أو `AGENT_CREDIT_EXHAUSTED` (نفاد الرصيد)، و`message` نص ثابت قد يذكر وقت تجدد الحد، و`classified` عدد المهارات المحفوظة، و`remaining` معرّفات المهارات التي لم تُصنَّف بترتيبها. المهارات المكتملة تُحفظ في commit واحد (`committed: true`) ولا يُمسح شيء منها؛ المهارة الجارية لحظة التوقف والباقي لا تُمس، ولا يُحفظ لها تصنيف كلمات مفتاحية. `agent_calls` يعدّ كل استدعاء نموذج بدأ، ومنها المقطوع. رمز الخروج 0، ويُكتب سبب التوقف إلى stderr. أخطاء Claude الأخرى (المهلة، والرد غير الصالح) ترجع للكلمات المفتاحية كما كانت.

**`doctor --repair` (قرار المالك، الجولتان الرابعة والسادسة):** شكل جديد `DoctorRepair`، ضمن النسخة 8. بدون `--confirm` (أو مع `--dry-run`) معاينة لا تكتب شيئًا: `dry_run: true`، والقوائم كاملة بلا حد عشرة: `restore` (مسارات في آخر commit تخالفه في الفهرس أو على القرص، تُعاد منه)، و`unstage` (مُرحَّلة وليست فيه، تُزال من الفهرس وتبقى على القرص)، و`recreate_directories` (مجلدات فارغة يسجلها ملف meta في آخر commit)، و`staging_removed` (مجلدات التجهيز `.skillsbox-`، تُنقل إلى النسخة الاحتياطية ولا تُحذف)، و`blocked` و`untracked` لا تُمس. `preview_token` بصيغة `YYYYMMDDTHHMMSSZ.<64 hex>`، وفارغ حين لا شيء يُصلح؛ و`backup_directory` هو `~/.kinana/repair-backups/<الوقت نفسه>`، و`null` حين لا شيء يُصلح. `--confirm <token>` يأخذ قفل الكاتب ويعيد بناء الخطة؛ أي تغير في آخر commit أو فهرس Git أو بايتات أي مسار في الخطة أو أنواعها أو صلاحياتها، أو رمز ليس بهذه الصيغة، يُرفض بـ`DOCTOR_REPAIR_STALE` ورمز الخروج 2 دون أي كتابة. ثم قبل أن يُمس المخزن تُنشأ `~/.kinana` إن لم توجد (0700)، وتُنسخ إلى `backup_directory` الملفات المتتبعة في `restore` الموجودة على القرص بمساراتها تحت `vault/`، والمحتوى الذي لا يحمله إلا فهرس Git (مُرحَّل يختلف عن آخر commit) لكل مسار في `restore` و`unstage` تحت `index/`، وملف `.git/index` باسم `git-index`، ومجلدات التجهيز تحت `stages/` (الروابط تُنسخ روابط ولا تُتبع)، مع `MANIFEST.txt` بتجزئة كل ملف وصلاحياته الأصلية؛ المجلدات 0700 والملفات 0600، وكل نسخة يُتحقق منها. فشل أي خطوة فيها `DOCTOR_REPAIR_BACKUP_FAILED` ورمز الخروج 2 والمخزن كما هو. بعدها تصل المسارات إلى `git restore` عبر `--pathspec-from-file` بفواصل NUL و`--literal-pathspecs`، بلا صدفة، ثم تُنشأ المجلدات الفارغة، وتُحذف مجلدات التجهيز بعد نسخها، ويُعاد بناء فهرس المخزن إن تغيّر ملف. فشل Git في منتصفها `GIT_ERROR` برسالة تذكر مجلد النسخة. الناتج `dry_run: false`، و`backed_up` الملفات المنسوخة، و`after` تقرير `doctor` كاملًا بعد الإصلاح، ورمز الخروج 0. لا commit جديد، ولا يحذف Kinana مجلد النسخة. لا يحذف ملفًا لا يتتبعه Git خارج مجلدات التجهيز ولا يغيّر سجل Git؛ ملفات داخل مجلد تجهيز تنتقل معه ولا تظهر في `untracked`. مجلد تجهيز يُعدّ فقط داخل مجلد حقيقي (جذر المخزن و`dist/` و`.git/` حين لا تكون روابط)، ومجلد تجهيز هو رابط يُنقل رابطًا ولا يُقرأ ما يشير إليه. أي فشل بعد أن يُمس المخزن يذكر مجلد النسخة ويطلب `kinana doctor`. `--repair` لا يجتمع مع `--rebuild-index`، و`--confirm` يتطلب `--repair`. الناتج النصي يذكر كل مسار بصيغة الهروب الموحّدة ويختم بسطر `kinana doctor --repair --confirm <token>` وحده.

**اقتراح الوكيل المنتظر:** `stack` و`stage` يبقيان في `metadata.proposal` في JSON كما وُثّقا في إصدار6. تخزينهما في ملف meta صار في جدول `[proposal_dimensions]` بجوار `[proposal]`، حتى يقرأ بيتا 0.1.0 المخزن والاقتراح منتظر (§3 من SPEC.md). الجدول ليس جزءًا من أي ناتج JSON.

يحسب التصنيف تكلفة سياق التداخل لكل مهارة بحسب فئتها. يعرض `fixed_tokens_total` مجموع تكاليف الطلبات و`fixed_tokens_per_call` متوسطها مقربًا للأعلى.
