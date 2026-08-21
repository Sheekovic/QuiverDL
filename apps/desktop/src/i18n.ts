export type Language = "en" | "ar";

const messages = {
  en: {
    downloads: "Downloads", active: "Active", completed: "History", attention: "Needs attention",
    settings: "Settings", newDownload: "NEW DOWNLOAD", pasteLink: "Paste a direct HTTP or HTTPS link",
    downloadUrl: "Download URL", inspect: "Inspect link", inspecting: "Inspecting...",
    choose: "Choose location and download", opening: "Opening...", empty: "Your queue is empty",
    emptyHint: "Paste a direct link above, inspect it, and choose where to save the file.",
    theme: "Theme", language: "Language", retryAttempts: "Retry attempts",
    connectionsDownload: "Connections per download", connectionsServer: "Connections per server",
    notifications: "Completion notifications", privateDesign: "Private by design",
    noTelemetry: "No accounts. No telemetry.",
    historyRetention: "Keep completed history", keepForever: "Forever",
    historyRetentionHint: "Legacy entries without a completion date remain until you remove them.",
    keepSevenDays: "7 days", keepThirtyDays: "30 days", keepNinetyDays: "90 days",
    searchHistory: "Search completed downloads", sortHistory: "Sort completed downloads",
    newestFirst: "Newest first", oldestFirst: "Oldest first", nameSort: "Name",
    sizeSort: "Largest first", clearCompleted: "Clear history", downloadHistory: "Download history", of: "of",
    item: "item", items: "items", noHistory: "No completed downloads",
    noHistoryHint: "Completed downloads will appear here and remain private on this device.",
    noHistoryMatch: "No completed downloads match your search.",
    clearCompletedConfirm: "Remove all completed downloads from history? Downloaded files will not be deleted.",
    completedOn: "Completed", completionDateUnavailable: "Completion date unavailable",
    removeFromHistory: "Remove from history", remove: "Remove",
  },
  ar: {
    downloads: "التنزيلات", active: "النشطة", completed: "السجل", attention: "تحتاج إلى انتباه",
    settings: "الإعدادات", newDownload: "تنزيل جديد", pasteLink: "الصق رابط HTTP أو HTTPS مباشرًا",
    downloadUrl: "رابط التنزيل", inspect: "فحص الرابط", inspecting: "جارٍ الفحص...",
    choose: "اختر الموقع وابدأ التنزيل", opening: "جارٍ الفتح...", empty: "قائمة التنزيل فارغة",
    emptyHint: "الصق رابطًا مباشرًا أعلاه، افحصه، ثم اختر مكان حفظ الملف.",
    theme: "المظهر", language: "اللغة", retryAttempts: "محاولات إعادة التنزيل",
    connectionsDownload: "الاتصالات لكل تنزيل", connectionsServer: "الاتصالات لكل خادم",
    notifications: "إشعارات اكتمال التنزيل", privateDesign: "خصوصية من الأساس",
    noTelemetry: "بلا حسابات أو تتبع.",
    historyRetention: "الاحتفاظ بسجل التنزيلات المكتملة", keepForever: "دائمًا",
    historyRetentionHint: "تظل السجلات القديمة التي لا تحتوي على تاريخ اكتمال حتى تزيلها.",
    keepSevenDays: "7 أيام", keepThirtyDays: "30 يومًا", keepNinetyDays: "90 يومًا",
    searchHistory: "البحث في التنزيلات المكتملة", sortHistory: "ترتيب التنزيلات المكتملة",
    newestFirst: "الأحدث أولًا", oldestFirst: "الأقدم أولًا", nameSort: "الاسم",
    sizeSort: "الأكبر أولًا", clearCompleted: "مسح السجل", downloadHistory: "سجل التنزيلات", of: "من",
    item: "عنصر", items: "عناصر", noHistory: "لا توجد تنزيلات مكتملة",
    noHistoryHint: "ستظهر التنزيلات المكتملة هنا وتظل خاصة على هذا الجهاز.",
    noHistoryMatch: "لا توجد تنزيلات مكتملة تطابق البحث.",
    clearCompletedConfirm: "هل تريد إزالة كل التنزيلات المكتملة من السجل؟ لن تُحذف الملفات التي تم تنزيلها.",
    completedOn: "اكتمل", completionDateUnavailable: "تاريخ الاكتمال غير متاح",
    removeFromHistory: "إزالة من السجل", remove: "إزالة",
  },
} as const;

export type MessageKey = keyof typeof messages.en;

export function translate(language: Language, key: MessageKey): string {
  return messages[language][key] ?? messages.en[key];
}
