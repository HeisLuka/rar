import type { CheckRecord, Compatibility } from './checks';
import type { SupportedLocale } from './i18n';

function html(value: string) {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#039;');
}

type EmailCopy = {
  subject:string; preheader:string; title:string; compatibility:string; summary:string;
  publisherFamily:string; pages:string; limitations:string; noLimitations:string;
  bounded:string; retention:string;
  compatibilityLabels:Record<Compatibility,string>;
  summaries:Record<string,string>;
};

const en:EmailCopy={
  subject:'Your Chaptera PUB compatibility report',
  preheader:'Chaptera PUB compatibility check',
  title:'Your report is ready',
  compatibility:'Compatibility',
  summary:'Summary',
  publisherFamily:'Publisher family',
  pages:'Pages observed',
  limitations:'Known limitations',
  noLimitations:'No additional limitations were included in this receipt.',
  bounded:'This is a bounded compatibility report, not a promise of perfect Publisher fidelity. Unknown or unsupported content remains explicitly unknown or unsupported.',
  retention:'The uploaded file is temporary and is deleted after the configured retention window.',
  compatibilityLabels:{compatible:'Compatible',partial:'Partial',unsupported:'Unsupported',invalid:'Invalid file',failed:'Check failed'},
  summaries:{
    'pub_check.viewer_supported':'Chaptera opened this file without a known fidelity warning inside the current support profile. This is not a claim of pixel-perfect Publisher rendering.',
    'pub_check.viewer_partial':'Chaptera opened this file, but one or more known fidelity limitations remain.',
    'pub_check.viewer_unsupported':'The current Reader pipeline does not admit this file as supported.',
    'pub_check.not_pub':'The uploaded bytes do not look like a supported Microsoft Publisher document.',
    'pub_check.damaged':'The file contains Publisher evidence, but its container or Contents data appears damaged or incomplete.',
    'pub_check.archive_with_pub':'The upload appears to be an archive containing a Publisher candidate rather than one directly supported PUB document.',
    'pub_check.recognized_but_unopenable':'This looks like a Publisher file, but the current Reader pipeline could not open it reliably enough to claim compatibility.',
    'pub_check.possible_pub':'The file may be Publisher-related, but there is not enough evidence to classify it as working.',
    'pub_check.internal_failure':'The checker could not complete this request reliably.',
    'pub_check.worker_failure':'The checker worker could not complete this request reliably.',
  },
};

const ru:EmailCopy={...en,
  subject:'Результат проверки вашего файла PUB — Chaptera',
  preheader:'Проверка совместимости PUB в Chaptera',
  title:'Отчёт о совместимости готов',
  compatibility:'Совместимость',
  summary:'Результат',
  publisherFamily:'Версия / семейство Publisher',
  pages:'Обнаружено страниц',
  limitations:'Известные ограничения',
  noLimitations:'Дополнительных ограничений в отчёте не указано.',
  bounded:'Это ограниченный отчёт о текущей совместимости, а не обещание идеального совпадения с Microsoft Publisher. Неизвестные и неподдерживаемые элементы остаются явно отмеченными как неизвестные или неподдерживаемые.',
  retention:'Загруженный файл временный и автоматически удаляется после установленного срока хранения.',
  compatibilityLabels:{compatible:'Совместим',partial:'Частично совместим',unsupported:'Не поддерживается',invalid:'Некорректный файл',failed:'Ошибка проверки'},
  summaries:{
    'pub_check.viewer_supported':'Chaptera открыл этот файл без известных предупреждений о точности отображения в рамках текущего профиля поддержки. Это не означает полного пиксельного совпадения с Microsoft Publisher.',
    'pub_check.viewer_partial':'Chaptera открыл этот файл, но для него остаются одно или несколько известных ограничений отображения или совместимости.',
    'pub_check.viewer_unsupported':'Текущая версия Reader не считает этот файл поддерживаемым.',
    'pub_check.not_pub':'Загруженные данные не похожи на поддерживаемый документ Microsoft Publisher.',
    'pub_check.damaged':'В файле обнаружены признаки Publisher, но контейнер или внутренние данные выглядят повреждёнными либо неполными.',
    'pub_check.archive_with_pub':'Похоже, загружен архив с файлом Publisher внутри, а не непосредственно поддерживаемый документ PUB.',
    'pub_check.recognized_but_unopenable':'Файл похож на документ Publisher, но текущая версия Reader не смогла открыть его достаточно надёжно, чтобы подтвердить совместимость.',
    'pub_check.possible_pub':'Файл может относиться к Publisher, но данных недостаточно, чтобы считать его рабочим.',
    'pub_check.internal_failure':'Не удалось надёжно завершить проверку этого файла.',
    'pub_check.worker_failure':'Сервис проверки не смог надёжно завершить обработку файла.',
  },
};

const fr:EmailCopy={...en,
  subject:'Votre rapport de compatibilité PUB — Chaptera',
  preheader:'Vérification de compatibilité PUB Chaptera', title:'Votre rapport est prêt',
  compatibility:'Compatibilité',summary:'Résultat',publisherFamily:'Famille Publisher',pages:'Pages détectées',
  limitations:'Limites connues',noLimitations:'Aucune limite supplémentaire n’a été incluse dans ce rapport.',
  bounded:'Ce rapport décrit la compatibilité actuellement démontrée ; il ne garantit pas une fidélité parfaite à Microsoft Publisher. Les éléments inconnus ou non pris en charge restent explicitement signalés.',
  retention:'Le fichier envoyé est temporaire et sera supprimé automatiquement après la période de conservation.',
  compatibilityLabels:{compatible:'Compatible',partial:'Partiel',unsupported:'Non pris en charge',invalid:'Fichier invalide',failed:'Échec du test'},
  summaries:{
    'pub_check.viewer_supported':'Chaptera a ouvert ce fichier sans avertissement de fidélité connu dans le profil actuellement pris en charge. Cela ne garantit pas un rendu identique au pixel près à Microsoft Publisher.',
    'pub_check.viewer_partial':'Chaptera a ouvert ce fichier, mais une ou plusieurs limites de fidélité connues subsistent.',
    'pub_check.viewer_unsupported':'La version actuelle du Reader ne considère pas ce fichier comme pris en charge.',
    'pub_check.not_pub':'Le fichier envoyé ne ressemble pas à un document Microsoft Publisher pris en charge.',
    'pub_check.damaged':'Des éléments Publisher sont présents, mais le conteneur ou les données semblent endommagés ou incomplets.',
    'pub_check.archive_with_pub':'Le fichier envoyé semble être une archive contenant un document Publisher, et non un fichier PUB directement pris en charge.',
    'pub_check.recognized_but_unopenable':'Le fichier semble provenir de Publisher, mais le Reader actuel ne peut pas l’ouvrir de façon assez fiable pour confirmer sa compatibilité.',
    'pub_check.possible_pub':'Le fichier peut être lié à Publisher, mais les preuves sont insuffisantes pour le déclarer fonctionnel.',
    'pub_check.internal_failure':'Le vérificateur n’a pas pu terminer cette demande de façon fiable.',
    'pub_check.worker_failure':'Le service de vérification n’a pas pu terminer cette demande de façon fiable.',
  },
};

const es:EmailCopy={...en,
  subject:'Tu informe de compatibilidad PUB — Chaptera',preheader:'Comprobación de compatibilidad PUB de Chaptera',title:'Tu informe está listo',
  compatibility:'Compatibilidad',summary:'Resultado',publisherFamily:'Familia Publisher',pages:'Páginas detectadas',limitations:'Limitaciones conocidas',
  noLimitations:'Este informe no incluye limitaciones adicionales.',
  bounded:'Este es un informe limitado a la compatibilidad que el verificador puede demostrar actualmente; no garantiza una reproducción perfecta de Microsoft Publisher. Lo desconocido o no compatible se indica expresamente.',
  retention:'El archivo subido es temporal y se elimina automáticamente después del periodo de conservación.',
  compatibilityLabels:{compatible:'Compatible',partial:'Parcial',unsupported:'No compatible',invalid:'Archivo no válido',failed:'Error de comprobación'},
  summaries:{
    'pub_check.viewer_supported':'Chaptera abrió este archivo sin avisos de fidelidad conocidos dentro del perfil actualmente compatible. No implica una reproducción idéntica a Microsoft Publisher.',
    'pub_check.viewer_partial':'Chaptera abrió este archivo, pero quedan una o más limitaciones de fidelidad conocidas.',
    'pub_check.viewer_unsupported':'El Reader actual no admite este archivo como compatible.',
    'pub_check.not_pub':'Los datos subidos no parecen un documento Microsoft Publisher compatible.',
    'pub_check.damaged':'Hay indicios de Publisher, pero el contenedor o sus datos parecen dañados o incompletos.',
    'pub_check.archive_with_pub':'La subida parece ser un archivo comprimido que contiene un candidato Publisher, no un PUB directamente compatible.',
    'pub_check.recognized_but_unopenable':'Parece un archivo Publisher, pero el Reader actual no puede abrirlo con suficiente fiabilidad para afirmar compatibilidad.',
    'pub_check.possible_pub':'El archivo podría estar relacionado con Publisher, pero no hay evidencia suficiente para considerarlo funcional.',
    'pub_check.internal_failure':'El verificador no pudo completar esta solicitud de forma fiable.',
    'pub_check.worker_failure':'El proceso de verificación no pudo completar esta solicitud de forma fiable.',
  },
};

const it:EmailCopy={...en,
  subject:'Il tuo report di compatibilità PUB — Chaptera',preheader:'Verifica compatibilità PUB di Chaptera',title:'Il report è pronto',
  compatibility:'Compatibilità',summary:'Risultato',publisherFamily:'Famiglia Publisher',pages:'Pagine rilevate',limitations:'Limitazioni note',
  noLimitations:'Il report non contiene ulteriori limitazioni.',
  bounded:'Questo report descrive solo la compatibilità attualmente dimostrabile e non garantisce una resa perfetta rispetto a Microsoft Publisher. Gli elementi sconosciuti o non supportati restano indicati esplicitamente.',
  retention:'Il file caricato è temporaneo e viene eliminato automaticamente dopo il periodo di conservazione.',
  compatibilityLabels:{compatible:'Compatibile',partial:'Parziale',unsupported:'Non supportato',invalid:'File non valido',failed:'Errore di verifica'},
  summaries:{
    'pub_check.viewer_supported':'Chaptera ha aperto questo file senza avvisi di fedeltà noti nel profilo attualmente supportato. Non è una garanzia di resa identica a Microsoft Publisher.',
    'pub_check.viewer_partial':'Chaptera ha aperto questo file, ma restano una o più limitazioni di fedeltà note.',
    'pub_check.viewer_unsupported':'Il Reader attuale non considera questo file supportato.',
    'pub_check.not_pub':'I dati caricati non sembrano un documento Microsoft Publisher supportato.',
    'pub_check.damaged':'Sono presenti elementi riconducibili a Publisher, ma il contenitore o i dati sembrano danneggiati o incompleti.',
    'pub_check.archive_with_pub':'Il caricamento sembra essere un archivio contenente un file Publisher, non un singolo PUB direttamente supportato.',
    'pub_check.recognized_but_unopenable':'Il file sembra provenire da Publisher, ma il Reader attuale non riesce ad aprirlo con affidabilità sufficiente per dichiararlo compatibile.',
    'pub_check.possible_pub':'Il file potrebbe essere collegato a Publisher, ma non ci sono prove sufficienti per dichiararlo funzionante.',
    'pub_check.internal_failure':'Il verificatore non ha potuto completare la richiesta in modo affidabile.',
    'pub_check.worker_failure':'Il processo di verifica non ha potuto completare la richiesta in modo affidabile.',
  },
};

const de:EmailCopy={...en,
  subject:'Ihr PUB-Kompatibilitätsbericht — Chaptera',preheader:'Chaptera PUB-Kompatibilitätsprüfung',title:'Ihr Bericht ist fertig',
  compatibility:'Kompatibilität',summary:'Ergebnis',publisherFamily:'Publisher-Familie',pages:'Erkannte Seiten',limitations:'Bekannte Einschränkungen',
  noLimitations:'Dieser Bericht enthält keine zusätzlichen Einschränkungen.',
  bounded:'Dieser Bericht beschreibt nur die aktuell nachweisbare Kompatibilität und ist keine Zusage perfekter Microsoft-Publisher-Fidelität. Unbekannte oder nicht unterstützte Inhalte bleiben ausdrücklich als solche gekennzeichnet.',
  retention:'Die hochgeladene Datei ist temporär und wird nach Ablauf der Aufbewahrungsfrist automatisch gelöscht.',
  compatibilityLabels:{compatible:'Kompatibel',partial:'Teilweise',unsupported:'Nicht unterstützt',invalid:'Ungültige Datei',failed:'Prüfung fehlgeschlagen'},
  summaries:{
    'pub_check.viewer_supported':'Chaptera hat diese Datei innerhalb des aktuellen Unterstützungsprofils ohne bekannte Fidelity-Warnung geöffnet. Das ist keine Zusage einer pixelgenauen Darstellung wie in Microsoft Publisher.',
    'pub_check.viewer_partial':'Chaptera hat diese Datei geöffnet, es bestehen jedoch eine oder mehrere bekannte Darstellungsbeschränkungen.',
    'pub_check.viewer_unsupported':'Der aktuelle Reader stuft diese Datei nicht als unterstützt ein.',
    'pub_check.not_pub':'Die hochgeladenen Daten sehen nicht wie ein unterstütztes Microsoft-Publisher-Dokument aus.',
    'pub_check.damaged':'Es gibt Hinweise auf Publisher, aber Container oder Inhaltsdaten wirken beschädigt oder unvollständig.',
    'pub_check.archive_with_pub':'Der Upload scheint ein Archiv mit einer Publisher-Datei zu sein und kein direkt unterstütztes PUB-Dokument.',
    'pub_check.recognized_but_unopenable':'Die Datei sieht nach Publisher aus, der aktuelle Reader kann sie jedoch nicht zuverlässig genug öffnen, um Kompatibilität zu bestätigen.',
    'pub_check.possible_pub':'Die Datei könnte zu Publisher gehören, aber die Evidenz reicht nicht aus, um sie als funktionsfähig einzustufen.',
    'pub_check.internal_failure':'Der Prüfer konnte die Anfrage nicht zuverlässig abschließen.',
    'pub_check.worker_failure':'Der Prüfprozess konnte die Anfrage nicht zuverlässig abschließen.',
  },
};

const emailCopy:Record<SupportedLocale,EmailCopy>={
  'en-US':en,'en-GB':en,'fr-FR':fr,'es-ES':es,'it-IT':it,'de-DE':de,'ru-RU':ru,
};

function copyFor(record:CheckRecord){
  return emailCopy[record.locale ?? 'en-US'] ?? en;
}
function localizedSummary(record:CheckRecord, copy:EmailCopy){
  const code=record.result?.diagnosticsCode;
  return (code && copy.summaries[code]) || record.result?.summary || copy.summaries['pub_check.internal_failure'];
}

export async function sendResultEmail(record: CheckRecord) {
  const apiKey = process.env.RESEND_API_KEY;
  const from = process.env.REPORT_FROM_EMAIL;
  if (!apiKey || !from) return 'not_configured' as const;
  if (!record.result) return 'failed' as const;

  const result = record.result;
  const copy=copyFor(record);
  const summary=localizedSummary(record,copy);
  const limitations =
    result.limitations?.length
      ? `<ul>${result.limitations.slice(0,12).map(item=>`<li>${html(item)}</li>`).join('')}</ul>`
      : `<p>${html(copy.noLimitations)}</p>`;

  const text = [
    copy.title,'',
    `${copy.compatibility}: ${copy.compatibilityLabels[result.compatibility]}`,
    `${copy.summary}: ${summary}`,
    result.publisherFamily ? `${copy.publisherFamily}: ${result.publisherFamily}` : '',
    typeof result.pages === 'number' ? `${copy.pages}: ${result.pages}` : '',
    result.limitations?.length ? `${copy.limitations}: ${result.limitations.join('; ')}` : `${copy.limitations}: ${copy.noLimitations}`,
    '',copy.bounded,copy.retention,
  ].filter(Boolean).join('\n');

  const payload = JSON.stringify({
    from,
    to: [record.email],
    subject: copy.subject,
    text,
    html: `
      <div lang="${html(record.locale ?? 'en-US')}" style="font-family:Inter,Arial,sans-serif;color:#172018;max-width:640px;margin:auto">
        <p style="font-size:13px;color:#617064">${html(copy.preheader)}</p>
        <h1 style="font-size:28px;line-height:1.15;margin:12px 0 18px">${html(copy.title)}</h1>
        <div style="border:1px solid #dfe7df;border-radius:16px;padding:18px">
          <p style="margin:0 0 8px;color:#617064;font-size:12px">${html(copy.compatibility)}</p>
          <p style="margin:0 0 18px;font-size:20px;font-weight:700">${html(copy.compatibilityLabels[result.compatibility])}</p>
          <p style="margin:0 0 8px;color:#617064;font-size:12px">${html(copy.summary)}</p>
          <p style="margin:0;line-height:1.55">${html(summary)}</p>
        </div>
        ${result.publisherFamily ? `<p><b>${html(copy.publisherFamily)}:</b> ${html(result.publisherFamily)}</p>` : ''}
        ${typeof result.pages === 'number' ? `<p><b>${html(copy.pages)}:</b> ${result.pages}</p>` : ''}
        <h2 style="font-size:17px;margin-top:24px">${html(copy.limitations)}</h2>
        ${limitations}
        <p style="font-size:12px;color:#617064;margin-top:26px;line-height:1.5">${html(copy.bounded)}</p>
        <p style="font-size:12px;color:#617064;line-height:1.5">${html(copy.retention)}</p>
      </div>
    `,
  });

  for (let attempt = 0; attempt < 2; attempt += 1) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 10000);
    try {
      const response = await fetch('https://api.resend.com/emails', {
        method: 'POST',
        headers: {
          authorization: `Bearer ${apiKey}`,
          'content-type': 'application/json',
          'idempotency-key': `pub-check-result/${record.id}`,
        },
        body: payload,
        signal: controller.signal,
      });
      if (response.ok) return 'sent' as const;
      if (attempt === 0 && (response.status === 429 || response.status >= 500)) continue;
      return 'failed' as const;
    } catch {
      if (attempt === 1) return 'failed' as const;
    } finally {
      clearTimeout(timer);
    }
  }
  return 'failed' as const;
}
