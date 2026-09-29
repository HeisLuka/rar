export const SUPPORTED_LOCALES = ['en-US','en-GB','fr-FR','es-ES','it-IT','de-DE','ru-RU'] as const;
export type SupportedLocale = typeof SUPPORTED_LOCALES[number];
export const SUPPORTED_COUNTRIES = ['US','GB','FR','ES','IT','DE','RU','INTL'] as const;
export type SupportedCountry = typeof SUPPORTED_COUNTRIES[number];

export const COUNTRY_OPTIONS = [
  ['US','United States'],['GB','United Kingdom'],['FR','France'],
  ['ES','España'],['IT','Italia'],['DE','Deutschland'],['RU','Россия'],['INTL','International'],
] as const;

export const LOCALE_OPTIONS = [
  ['en-US','English (US)'],['en-GB','English (UK)'],['fr-FR','Français'],
  ['es-ES','Español'],['it-IT','Italiano'],['de-DE','Deutsch'],['ru-RU','Русский'],
] as const;

const byCountry: Record<SupportedCountry, SupportedLocale> = {
  US:'en-US', GB:'en-GB', FR:'fr-FR', ES:'es-ES', IT:'it-IT', DE:'de-DE', RU:'ru-RU', INTL:'en-US',
};

export function normalizeCountry(v?: string | null): SupportedCountry | null {
  const x=v?.trim().toUpperCase();
  return x && (SUPPORTED_COUNTRIES as readonly string[]).includes(x) ? x as SupportedCountry : null;
}
export function normalizeLocale(v?: string | null): SupportedLocale | null {
  if (!v) return null;
  const x=v.trim().replace('_','-').toLowerCase();
  const exact=SUPPORTED_LOCALES.find(l=>l.toLowerCase()===x);
  if (exact) return exact;
  if (x.startsWith('fr')) return 'fr-FR';
  if (x.startsWith('es')) return 'es-ES';
  if (x.startsWith('it')) return 'it-IT';
  if (x.startsWith('de')) return 'de-DE';
  if (x.startsWith('ru')) return 'ru-RU';
  if (x.startsWith('en-gb')) return 'en-GB';
  if (x.startsWith('en')) return 'en-US';
  return null;
}
export function resolveCountry(input:{cookie?:string|null;vercel?:string|null}) {
  return normalizeCountry(input.cookie) ?? normalizeCountry(input.vercel) ?? 'INTL';
}
export function resolveLocale(input:{cookie?:string|null;accept?:string|null;country:SupportedCountry}) {
  if (normalizeLocale(input.cookie)) return normalizeLocale(input.cookie)!;
  for (const item of (input.accept ?? '').split(',')) {
    const locale=normalizeLocale(item.split(';')[0]);
    if (locale) return locale;
  }
  return byCountry[input.country];
}

export type Copy = {
  badge:string; region:string; language:string; detected:string;
  eyebrow:string; title:string; body:string;
  drop:string; dropHint:string; remove:string; email:string; submit:string; uploading:string; consent:string; progress:string;
  ready:string; failed:string; processing:string; queued:string; waiting:string;
  compatibility:string; publisherFamily:string; emailMetric:string; emailSent:string; emailPrepared:string; notIdentified:string;
  reliableFailure:string;
  privateTitle:string; privateBody:string; evidenceTitle:string; evidenceBody:string; emailTitle:string; emailBody:string;
  footerLeft:string; footerRight:string;
  errors:{pubOnly:string;fileSize:string;start:string;upload:string};
};

const en:Copy={
  badge:'PUB compatibility check · preview', region:'Region', language:'Language', detected:'Detected automatically',
  eyebrow:'Microsoft Publisher files', title:'Will your .PUB file still work?',
  body:'Drop in a Publisher file. Chaptera will inspect it in an isolated checker and email you a compatibility report with what we can read, what may be incomplete, and what needs attention.',
  drop:'Drop your .PUB file here', dropHint:'or click to choose a file · up to 64 MB', remove:'Remove',
  email:'you@company.com', submit:'Check my file', uploading:'Uploading…',
  consent:'I have the right to upload this file. It will be used only for this compatibility check and deleted automatically after the retention window.',
  progress:'Uploading privately', ready:'Compatibility report ready', failed:'We could not complete this check',
  processing:'Checking your Publisher file', queued:'Your file is in the checking queue',
  waiting:'You can close this page. We will send the result to the email address you provided.',
  compatibility:'Compatibility', publisherFamily:'Publisher family', emailMetric:'Email', emailSent:'Report sent', emailPrepared:'Report prepared',
  notIdentified:'Not identified', reliableFailure:'The checker could not produce a reliable result. We will not label the file compatible when the evidence is incomplete.',
  privateTitle:'Private by default', privateBody:'The uploaded file is private and is not exposed as a public download.',
  evidenceTitle:'Evidence, not guesses', evidenceBody:'Unknown or unsupported parts are reported instead of being silently treated as working.',
  emailTitle:'Results by email', emailBody:'The final compatibility summary is sent only after the checker returns a completed receipt.',
  footerLeft:'Chaptera · Publisher continuity tools', footerRight:'Uploaded files are temporary and used only to perform the requested check.',
  errors:{pubOnly:'Please choose a Microsoft Publisher .pub file.',fileSize:'The file must be between 1 byte and 64 MB.',start:'Could not start the check.',upload:'Upload failed.'}
};
const fr:Copy={...en,badge:'Test de compatibilité PUB · aperçu',region:'Région',language:'Langue',detected:'Détecté automatiquement',eyebrow:'Fichiers Microsoft Publisher',title:'Votre fichier .PUB est-il encore exploitable ?',body:'Déposez un fichier Publisher. Chaptera l’analyse dans un environnement isolé puis vous envoie par e-mail un rapport indiquant ce qui est lisible, partiel ou non pris en charge.',drop:'Déposez votre fichier .PUB ici',dropHint:'ou cliquez pour choisir un fichier · 64 Mo maximum',remove:'Retirer',email:'vous@entreprise.fr',submit:'Tester mon fichier',uploading:'Envoi…',consent:'Je confirme avoir le droit de téléverser ce fichier. Il sera utilisé uniquement pour ce test puis supprimé automatiquement.',progress:'Envoi privé',ready:'Rapport de compatibilité prêt',failed:'Le test n’a pas pu être terminé',processing:'Analyse de votre fichier Publisher',queued:'Votre fichier est dans la file de vérification',waiting:'Vous pouvez fermer cette page. Le résultat sera envoyé à l’adresse e-mail indiquée.',compatibility:'Compatibilité',publisherFamily:'Famille Publisher',emailMetric:'E-mail',emailSent:'Rapport envoyé',emailPrepared:'Rapport préparé',notIdentified:'Non identifiée',reliableFailure:'Le vérificateur n’a pas obtenu de résultat suffisamment fiable.',privateTitle:'Privé par défaut',privateBody:'Le fichier envoyé reste privé et n’est jamais publié comme téléchargement public.',evidenceTitle:'Des preuves, pas des suppositions',evidenceBody:'Les éléments inconnus ou non pris en charge sont signalés explicitement.',emailTitle:'Résultat par e-mail',emailBody:'Le rapport final est envoyé uniquement après un résultat complet du vérificateur.',footerLeft:'Chaptera · continuité des fichiers Publisher',footerRight:'Les fichiers envoyés sont temporaires et utilisés uniquement pour le test demandé.',errors:{pubOnly:'Choisissez un fichier Microsoft Publisher .pub.',fileSize:'Le fichier doit faire entre 1 octet et 64 Mo.',start:'Impossible de démarrer le test.',upload:'Échec de l’envoi.'}};
const es:Copy={...en,badge:'Comprobación PUB · vista previa',region:'Región',language:'Idioma',detected:'Detectado automáticamente',eyebrow:'Archivos de Microsoft Publisher',title:'¿Tu archivo .PUB sigue funcionando?',body:'Suelta un archivo de Publisher. Chaptera lo analizará de forma aislada y te enviará por correo un informe claro de compatibilidad.',drop:'Suelta aquí tu archivo .PUB',dropHint:'o haz clic para elegirlo · hasta 64 MB',remove:'Quitar',email:'tu@empresa.es',submit:'Comprobar archivo',uploading:'Subiendo…',consent:'Confirmo que tengo derecho a subir este archivo. Se usará solo para esta comprobación y se eliminará automáticamente.',progress:'Subida privada',ready:'Informe de compatibilidad listo',failed:'No pudimos completar la comprobación',processing:'Comprobando tu archivo de Publisher',queued:'Tu archivo está en la cola de comprobación',waiting:'Puedes cerrar esta página. Enviaremos el resultado al correo indicado.',compatibility:'Compatibilidad',publisherFamily:'Familia Publisher',emailMetric:'Correo',emailSent:'Informe enviado',emailPrepared:'Informe preparado',notIdentified:'No identificada',reliableFailure:'El verificador no pudo obtener un resultado fiable.',privateTitle:'Privado por defecto',privateBody:'El archivo subido es privado y no se publica como descarga.',evidenceTitle:'Pruebas, no suposiciones',evidenceBody:'Lo desconocido o no compatible se indica explícitamente.',emailTitle:'Resultado por correo',emailBody:'El informe final solo se envía cuando el verificador devuelve un resultado completo.',footerLeft:'Chaptera · continuidad para Publisher',footerRight:'Los archivos son temporales y se usan solo para la comprobación solicitada.',errors:{pubOnly:'Elige un archivo Microsoft Publisher .pub.',fileSize:'El archivo debe tener entre 1 byte y 64 MB.',start:'No se pudo iniciar la comprobación.',upload:'La subida ha fallado.'}};
const it:Copy={...en,badge:'Verifica compatibilità PUB · anteprima',region:'Paese',language:'Lingua',detected:'Rilevato automaticamente',eyebrow:'File Microsoft Publisher',title:'Il tuo file .PUB funziona ancora?',body:'Trascina un file Publisher. Chaptera lo analizzerà in modo isolato e ti invierà via e-mail un report di compatibilità.',drop:'Trascina qui il tuo file .PUB',dropHint:'oppure fai clic per sceglierlo · massimo 64 MB',remove:'Rimuovi',email:'tu@azienda.it',submit:'Verifica il file',uploading:'Caricamento…',consent:'Confermo di avere il diritto di caricare questo file. Verrà usato solo per questa verifica e poi eliminato automaticamente.',progress:'Caricamento privato',ready:'Report di compatibilità pronto',failed:'Non è stato possibile completare la verifica',processing:'Verifica del file Publisher',queued:'Il file è in coda per la verifica',waiting:'Puoi chiudere questa pagina. Invieremo il risultato all’indirizzo e-mail indicato.',compatibility:'Compatibilità',publisherFamily:'Famiglia Publisher',emailMetric:'E-mail',emailSent:'Report inviato',emailPrepared:'Report pronto',notIdentified:'Non identificata',reliableFailure:'Il verificatore non ha prodotto un risultato affidabile.',privateTitle:'Privato per impostazione predefinita',privateBody:'Il file caricato resta privato.',evidenceTitle:'Prove, non supposizioni',evidenceBody:'Gli elementi sconosciuti o non supportati vengono segnalati chiaramente.',emailTitle:'Risultato via e-mail',emailBody:'Il report finale viene inviato solo dopo un risultato completo.',footerLeft:'Chaptera · continuità per Publisher',footerRight:'I file caricati sono temporanei e usati solo per la verifica richiesta.',errors:{pubOnly:'Scegli un file Microsoft Publisher .pub.',fileSize:'Il file deve avere una dimensione compresa tra 1 byte e 64 MB.',start:'Impossibile avviare la verifica.',upload:'Caricamento non riuscito.'}};
const de:Copy={...en,badge:'PUB-Kompatibilitätsprüfung · Vorschau',region:'Region',language:'Sprache',detected:'Automatisch erkannt',eyebrow:'Microsoft-Publisher-Dateien',title:'Funktioniert Ihre .PUB-Datei noch?',body:'Laden Sie eine Publisher-Datei hoch. Chaptera prüft sie isoliert und sendet Ihnen per E-Mail einen Kompatibilitätsbericht.',drop:'.PUB-Datei hier ablegen',dropHint:'oder klicken und Datei auswählen · bis 64 MB',remove:'Entfernen',email:'sie@unternehmen.de',submit:'Datei prüfen',uploading:'Wird hochgeladen…',consent:'Ich bin berechtigt, diese Datei hochzuladen. Sie wird nur für diese Prüfung verarbeitet und danach automatisch gelöscht.',progress:'Privater Upload',ready:'Kompatibilitätsbericht ist fertig',failed:'Die Prüfung konnte nicht abgeschlossen werden',processing:'Publisher-Datei wird geprüft',queued:'Ihre Datei befindet sich in der Prüfwarteschlange',waiting:'Sie können diese Seite schließen. Das Ergebnis wird an die angegebene E-Mail-Adresse gesendet.',compatibility:'Kompatibilität',publisherFamily:'Publisher-Familie',emailMetric:'E-Mail',emailSent:'Bericht gesendet',emailPrepared:'Bericht erstellt',notIdentified:'Nicht erkannt',reliableFailure:'Der Prüfer konnte kein verlässliches Ergebnis erzeugen.',privateTitle:'Standardmäßig privat',privateBody:'Die hochgeladene Datei bleibt privat.',evidenceTitle:'Evidenz statt Vermutung',evidenceBody:'Unbekannte oder nicht unterstützte Bestandteile werden ausdrücklich gemeldet.',emailTitle:'Ergebnis per E-Mail',emailBody:'Der Abschlussbericht wird erst nach einem vollständigen Prüfergebnis versendet.',footerLeft:'Chaptera · Publisher-Dateien weiter nutzen',footerRight:'Hochgeladene Dateien sind temporär und werden ausschließlich für die angeforderte Prüfung verwendet.',errors:{pubOnly:'Bitte wählen Sie eine Microsoft-Publisher-Datei im Format .pub.',fileSize:'Die Datei muss zwischen 1 Byte und 64 MB groß sein.',start:'Die Prüfung konnte nicht gestartet werden.',upload:'Upload fehlgeschlagen.'}};

const ru:Copy={...en,badge:'Проверка PUB · предварительная версия',region:'Страна',language:'Язык',detected:'Определено автоматически',eyebrow:'Файлы Microsoft Publisher',title:'Будет ли работать ваш файл .PUB?',body:'Перетащите файл Publisher. Chaptera проверит его в изолированной среде и пришлёт на почту отчёт: что удалось прочитать, что отображается частично и что пока не поддерживается.',drop:'Перетащите файл .PUB сюда',dropHint:'или нажмите, чтобы выбрать файл · до 64 МБ',remove:'Убрать',email:'вы@компания.ру',submit:'Проверить файл',uploading:'Загрузка…',consent:'Я подтверждаю, что имею право загрузить этот файл. Он будет использован только для проверки совместимости и автоматически удалён после срока хранения.',progress:'Приватная загрузка',ready:'Отчёт о совместимости готов',failed:'Не удалось завершить проверку',processing:'Проверяем файл Publisher',queued:'Файл поставлен в очередь на проверку',waiting:'Эту страницу можно закрыть. Результат придёт на указанный адрес электронной почты.',compatibility:'Совместимость',publisherFamily:'Версия / семейство Publisher',emailMetric:'Почта',emailSent:'Отчёт отправлен',emailPrepared:'Отчёт подготовлен',notIdentified:'Не определено',reliableFailure:'Проверка не дала достаточно надёжного результата. Мы не будем считать файл совместимым, если данных недостаточно.',privateTitle:'Приватность по умолчанию',privateBody:'Загруженный файл остаётся приватным и не публикуется в открытом доступе.',evidenceTitle:'Факты, а не догадки',evidenceBody:'Неизвестные и неподдерживаемые элементы явно отмечаются, а не считаются рабочими автоматически.',emailTitle:'Результат на почту',emailBody:'Итоговый отчёт отправляется только после завершения реальной проверки файла.',footerLeft:'Chaptera · инструменты для файлов Publisher',footerRight:'Загруженные файлы временные и используются только для запрошенной проверки.',errors:{pubOnly:'Выберите файл Microsoft Publisher в формате .pub.',fileSize:'Размер файла должен быть от 1 байта до 64 МБ.',start:'Не удалось запустить проверку.',upload:'Не удалось загрузить файл.'}};

export const COPY:Record<SupportedLocale,Copy>={'en-US':en,'en-GB':{...en,title:'Can Chaptera still open your .PUB file?'},'fr-FR':fr,'es-ES':es,'it-IT':it,'de-DE':de,'ru-RU':ru};
