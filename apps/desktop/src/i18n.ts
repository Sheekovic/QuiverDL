export type Language = "en" | "ar";

const messages = {
  en: {
    downloads: "Downloads", active: "Active", completed: "Completed", attention: "Needs attention",
    settings: "Settings", newDownload: "NEW DOWNLOAD", pasteLink: "Paste a direct HTTP or HTTPS link",
    downloadUrl: "Download URL", inspect: "Inspect link", inspecting: "Inspecting...",
    choose: "Choose location and download", opening: "Opening...", empty: "Your queue is empty",
    emptyHint: "Paste a direct link above, inspect it, and choose where to save the file.",
    theme: "Theme", language: "Language", retryAttempts: "Retry attempts",
    connectionsDownload: "Connections per download", connectionsServer: "Connections per server",
    notifications: "Completion notifications", privateDesign: "Private by design",
    noTelemetry: "No accounts. No telemetry.",
  },
  ar: {
    downloads: "التنزيلات", active: "النشطة", completed: "المكتملة", attention: "تحتاج إلى انتباه",
    settings: "الإعدادات", newDownload: "تنزيل جديد", pasteLink: "الصق رابط HTTP أو HTTPS مباشرًا",
    downloadUrl: "رابط التنزيل", inspect: "فحص الرابط", inspecting: "جارٍ الفحص...",
    choose: "اختر الموقع وابدأ التنزيل", opening: "جارٍ الفتح...", empty: "قائمة التنزيل فارغة",
    emptyHint: "الصق رابطًا مباشرًا أعلاه، افحصه، ثم اختر مكان حفظ الملف.",
    theme: "المظهر", language: "اللغة", retryAttempts: "محاولات إعادة التنزيل",
    connectionsDownload: "الاتصالات لكل تنزيل", connectionsServer: "الاتصالات لكل خادم",
    notifications: "إشعارات اكتمال التنزيل", privateDesign: "خصوصية من الأساس",
    noTelemetry: "بلا حسابات أو تتبع.",
  },
} as const;

export type MessageKey = keyof typeof messages.en;

export function translate(language: Language, key: MessageKey): string {
  return messages[language][key] ?? messages.en[key];
}
