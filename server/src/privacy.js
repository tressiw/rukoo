// The privacy statement at /privacy, in English or Dutch. It is filled in from the operator's settings, so a fork
// that deploys this server gets one that names it; read it through and change what doesn't fit your setup before
// people sign up. Set PRIVACY_URL to use a statement of your own elsewhere instead.

import { escape, link } from './page.js';

// When the text below last changed in a way that matters to the people it describes.
export const UPDATED = '2026-11-11';
const PROTOCOL_URL = 'https://github.com/MajesteitBart/rukoo/blob/main/server/PROTOCOL.md';

const TEXT = {
  en: {
    title: 'Privacy statement for Rukoo accounts',
    updated: (date) => `Last updated ${date}.`,
    other: '<a href="/privacy?lang=nl">Nederlands</a>',
    who: 'Who is responsible',
    whoText: (op) => `${op} runs this server and decides what it does with your data.`,
    unnamed: 'Whoever runs this server hasn’t filled in their name. They decide what it does with your data.',
    contact: (c) => ` Contact: ${c}.`,
    what: 'What this service is',
    whatText:
      'A Rukoo account keeps your inbox categories, the category each email got and your corrections the same on your devices that use Rukoo, the open-source mail app. If you turn it on, it also runs your booking page, where other people and their AI assistants can book time with you. Your mail, your mail passwords and your Google sign-ins never pass through this server.',
    data: 'What we keep',
    dataItems: [
      'Your email address, which is your sign-in name. We don’t send mail to it and don’t check it.',
      'A check value derived from your password on your device. It lets the server confirm your password; it isn’t your password, and the server can’t turn it back into one.',
      'Your encrypted records. Your devices encrypt them with a key that only they have, so we can’t read them: not your categories, not your corrections, not which email went where.',
      'Your encrypted data key, wrapped by your password and by your recovery key. We can’t open it.',
      'The names and platforms of your devices, when each one signed in and when it last connected.',
      'How many records you have, how large they are and when they change.',
      'Failed sign-in attempts, to lock an account for 15 minutes after ten of them. Wrong passwords entered on a signed-in device count for that device alone.',
      'If you turn on a booking page: what it publishes and the bookings made on it, as Booking pages below describes.'
    ],
    ip: 'Your IP address reaches Cloudflare with every request. The server uses it to limit how often one address may try to sign in or write to a booking page, and doesn’t store it. For bookings it also counts how many came from one network that day (an IPv4 /24 or an IPv6 /64). It keeps those counts only as hashes made with a key that changes every day, and deletes them once the host’s day is over. Cloudflare may keep addresses in its own logs for a short time.',
    why: 'Why',
    whyText:
      'We use this data to give you the account you asked for: signing in, syncing your devices and showing your devices and usage (the agreement with you, article 6(1)(b) GDPR). Rate limits and the sign-in lock protect your account and the service (a legitimate interest, article 6(1)(f) GDPR). We don’t use it for anything else: no analytics, no advertising, no profiles, and we don’t sell or share it.',
    where: 'Where it is kept, and who helps',
    whereText: (place) =>
      `The server runs on Cloudflare, a service of Cloudflare, Inc. in the United States, which processes the data on our behalf under its data processing terms. ${place} Because Cloudflare is a US company, US law applies to it, also for data kept in the EU. What it could be made to hand over is the list above: your records stay encrypted with a key Cloudflare doesn’t have. A booking page is different: what it publishes is readable, and so are a guest’s details at the moment they book (see Booking pages).`,
    places: {
      eu: 'Accounts are stored and handled in the European Union.',
      us: 'Accounts are stored and handled in the United States.',
      fedramp: 'Accounts are stored and handled in FedRAMP-Moderate data centers in the United States.',
      any: 'Each account is stored in one Cloudflare location, usually near the device that made it.'
    },
    keep: 'How long',
    keepText:
      'We keep your account until you delete it. Delete account in Rukoo (Settings, Rukoo account) removes the account, every record and your booking page with its bookings at once. Cloudflare keeps backups of the storage that can bring back its state from up to 30 days before, so deleted data is fully gone after that. Inactive accounts aren’t deleted on their own; ask us if you want yours removed and can no longer sign in.',
    rights: 'Your rights',
    rightsText:
      'You can see what is stored about you on the What’s stored and shared page in Rukoo, export everything with Export my data, change your password and delete your account yourself. For anything else, such as a copy of the data we hold or an objection, contact us. You can also complain to a data protection authority, such as the one in the country where you live. If you booked time with someone through a booking page, Booking pages below says what you can do.',
    booking: 'Booking pages',
    bookingText: [
      'A booking page, at an address that ends in a name the account holder picks, lets other people and their AI assistants book time with that account holder: the host. It is off until the host turns it on in Rukoo. While it is on, this server keeps what the host publishes, readable, because it has to offer times while the host’s computers are off: the host’s name, introduction, time zone and language, the meeting types with their public locations, an optional contact line, the hours the host is open and the busy times within them (to the quarter hour, without titles or other details), and when the page was last updated. The page shows the open times, never the busy times themselves.',
      'If you book through a booking page, you are a guest. You give your name, your email address, an optional note and, when an assistant books for you, the assistant’s name. They go to the host: the host’s Rukoo puts the meeting in the host’s calendar, and the host’s calendar and mail provider, such as Google, sends you the invitation. From then on the host decides what happens with your details. The host’s Rukoo keeps its own copy, end-to-end encrypted, and deletes it 30 days after the meeting; the meeting itself stays in the host’s calendar like any other.',
      'We handle a guest’s details to make the booking the guest asks for, on the host’s behalf (steps taken at the guest’s request, article 6(1)(b) GDPR). Limits on how often and from where bookings come protect hosts and the service (a legitimate interest, article 6(1)(f) GDPR).',
      'This server reads a guest’s details once, when the booking is made, to seal them: it encrypts them with a key that only the host’s devices hold, and keeps only that sealed copy. A later leak of the storage, a backup or a break-in afterwards doesn’t reveal them. But the details do pass through the server readable at the moment of booking, so the operator, or someone who has broken into the server at that moment, could read them then. Next to the sealed copy the server keeps the booking’s time, meeting type and status, how it was made (the web page, the JSON API or MCP), a hash of the key in the guest’s manage link, and, while the booking is active, a hash of the guest’s email address bound to that time, so the same booking isn’t made twice.',
      'A booking stays on this server until 30 days after the meeting. When the guest cancels, the host declines or a request lapses, the sealed details and the email hash are deleted at once; the rest goes 30 days later. When the host turns the page off or deletes their account, the page and all its bookings are deleted.',
      'A guest can cancel the booking with the link on the page they saw after booking; that deletes their details from this server straight away. For what the host does with them, contact the host. For anything about the copy on this server, contact us.',
      'Hosts write their own pages; we don’t check them. To report a page that pretends to be someone else or is abusive, use Report this page at the bottom of it, or contact us.'
    ],
    security: 'Security',
    securityText: (protocol) =>
      `Your password goes through Argon2id on your device, and your records are encrypted with AES-256-GCM before they leave it. We can’t reset your password: without your password or your recovery key, nobody can open your data, including us. ${protocol} describes exactly how it works.`,
    protocol: 'The Rukoo account protocol',
    changes: 'Changes',
    changesText: 'When this statement changes in a way that matters to you, the date at the top changes too.'
  },
  nl: {
    title: 'Privacyverklaring voor Rukoo-accounts',
    updated: (date) => `Laatst bijgewerkt op ${date}.`,
    other: '<a href="/privacy?lang=en">English</a>',
    who: 'Wie is verantwoordelijk',
    whoText: (op) => `${op} beheert deze server en bepaalt wat er met je gegevens gebeurt.`,
    unnamed: 'De beheerder van deze server heeft geen naam ingevuld. Die bepaalt wat er met je gegevens gebeurt.',
    contact: (c) => ` Contact: ${c}.`,
    what: 'Wat deze dienst is',
    whatText:
      'Een Rukoo-account houdt je inboxcategorieën, de categorie van elke e-mail en je correcties gelijk op je apparaten met Rukoo, de opensource-mailapp. Als je het aanzet, draait het ook je afsprakenpagina, waar anderen en hun AI-assistenten een afspraak met je kunnen maken. Je e-mail, je e-mailwachtwoorden en je Google-aanmeldingen gaan nooit door deze server.',
    data: 'Wat we bewaren',
    dataItems: [
      'Je e-mailadres, waarmee je inlogt. We sturen er geen mail naar en controleren het niet.',
      'Een controlewaarde die je apparaat uit je wachtwoord afleidt. Daarmee kan de server je wachtwoord controleren; het is niet je wachtwoord, en de server kan er geen wachtwoord van terugmaken.',
      'Je versleutelde records. Je apparaten versleutelen ze met een sleutel die alleen zij hebben, dus wij kunnen ze niet lezen: niet je categorieën, niet je correcties, niet welke e-mail waar terechtkwam.',
      'Je versleutelde gegevenssleutel, ingepakt met je wachtwoord en met je herstelsleutel. Wij kunnen hem niet openen.',
      'De namen en platforms van je apparaten, wanneer elk ervan inlogde en wanneer het voor het laatst verbinding had.',
      'Hoeveel records je hebt, hoe groot ze zijn en wanneer ze veranderen.',
      'Mislukte inlogpogingen, om een account na tien daarvan 15 minuten te blokkeren. Verkeerde wachtwoorden op een ingelogd apparaat tellen alleen voor dat apparaat.',
      'Als je een afsprakenpagina aanzet: wat die publiceert en de afspraken die erop worden gemaakt, zoals hieronder bij Afsprakenpagina’s staat.'
    ],
    ip: 'Bij elk verzoek komt je IP-adres bij Cloudflare binnen. De server gebruikt het om te beperken hoe vaak één adres mag proberen in te loggen of iets naar een afsprakenpagina mag sturen, en slaat het niet op. Voor afspraken telt de server ook hoeveel er die dag van één netwerk kwamen (een IPv4-/24 of een IPv6-/64). Die tellingen bewaart hij alleen als hashes, gemaakt met een sleutel die elke dag wisselt, en hij verwijdert ze zodra de dag van de host voorbij is. Cloudflare kan adressen kort in zijn eigen logs bewaren.',
    why: 'Waarom',
    whyText:
      'We gebruiken deze gegevens om je het account te geven waar je om vroeg: inloggen, je apparaten synchroniseren en je apparaten en gebruik tonen (de overeenkomst met jou, artikel 6 lid 1 onder b AVG). Snelheidslimieten en de inlogblokkade beschermen je account en de dienst (een gerechtvaardigd belang, artikel 6 lid 1 onder f AVG). We gebruiken ze nergens anders voor: geen analyses, geen advertenties, geen profielen, en we verkopen of delen ze niet.',
    where: 'Waar het staat, en wie helpt',
    whereText: (place) =>
      `De server draait op Cloudflare, een dienst van Cloudflare, Inc. in de Verenigde Staten, dat de gegevens namens ons verwerkt onder zijn verwerkersvoorwaarden. ${place} Omdat Cloudflare een Amerikaans bedrijf is, valt het onder Amerikaans recht, ook voor gegevens in de EU. Wat het zou kunnen moeten afgeven, is de lijst hierboven: je records blijven versleuteld met een sleutel die Cloudflare niet heeft. Een afsprakenpagina is anders: wat die publiceert is leesbaar, en de gegevens van een gast ook, op het moment dat die boekt (zie Afsprakenpagina’s).`,
    places: {
      eu: 'Accounts worden opgeslagen en verwerkt in de Europese Unie.',
      us: 'Accounts worden opgeslagen en verwerkt in de Verenigde Staten.',
      fedramp: 'Accounts worden opgeslagen en verwerkt in FedRAMP-Moderate-datacenters in de Verenigde Staten.',
      any: 'Elk account staat op één Cloudflare-locatie, meestal dicht bij het apparaat dat het maakte.'
    },
    keep: 'Hoe lang',
    keepText:
      'We bewaren je account tot je het verwijdert. Account verwijderen in Rukoo (Instellingen, Rukoo-account) verwijdert het account, alle records en je afsprakenpagina met de afspraken erop in één keer. Cloudflare bewaart back-ups van de opslag waarmee de toestand van tot 30 dagen eerder terug te halen is, dus na die tijd zijn verwijderde gegevens helemaal weg. Inactieve accounts worden niet vanzelf verwijderd; vraag het ons als je het jouwe weg wilt hebben en niet meer kunt inloggen.',
    rights: 'Je rechten',
    rightsText:
      'Je ziet wat er over je is opgeslagen op de pagina Wat er wordt opgeslagen en gedeeld in Rukoo, je exporteert alles met Mijn gegevens exporteren, en je wijzigt je wachtwoord en verwijdert je account zelf. Voor al het andere, zoals een kopie van de gegevens die we hebben of een bezwaar, neem je contact met ons op. Je kunt ook een klacht indienen bij een toezichthouder, zoals de Autoriteit Persoonsgegevens of die in het land waar je woont. Heb je via een afsprakenpagina een afspraak met iemand gemaakt? Onder Afsprakenpagina’s staat wat je kunt doen.',
    booking: 'Afsprakenpagina’s',
    bookingText: [
      'Met een afsprakenpagina, op een adres dat eindigt op een naam die de accounthouder kiest, kunnen anderen en hun AI-assistenten een afspraak maken met die accounthouder: de host. De pagina staat uit tot de host hem in Rukoo aanzet. Zolang hij aanstaat, bewaart deze server leesbaar wat de host publiceert, omdat hij ook tijden moet kunnen aanbieden als de computers van de host uitstaan: de naam van de host, de introductie, tijdzone en taal, de soorten afspraken met hun openbare locaties, een eventueel contactbericht, de uren waarop de host beschikbaar is en de bezette tijden daarbinnen (per kwartier, zonder titels of andere details), en wanneer de pagina voor het laatst is bijgewerkt. De pagina laat de vrije tijden zien, nooit de bezette tijden zelf.',
      'Boek je via een afsprakenpagina, dan ben je gast. Je geeft je naam, je e-mailadres, eventueel een opmerking en, als een assistent voor je boekt, de naam van die assistent. Die gaan naar de host: Rukoo zet de afspraak in de agenda van de host, en de agenda- en mailprovider van de host, zoals Google, stuurt je de uitnodiging. Daarna bepaalt de host wat er met je gegevens gebeurt. Rukoo bewaart bij de host een eigen kopie, end-to-end versleuteld, en verwijdert die 30 dagen na de afspraak; de afspraak zelf blijft in de agenda van de host staan, zoals elke andere.',
      'We verwerken de gegevens van een gast om de afspraak te maken waar de gast om vraagt, namens de host (stappen op verzoek van de gast, artikel 6 lid 1 onder b AVG). Limieten op hoe vaak en vanaf waar er wordt geboekt, beschermen hosts en de dienst (een gerechtvaardigd belang, artikel 6 lid 1 onder f AVG).',
      'Deze server leest de gegevens van een gast één keer, bij het boeken, om ze te verzegelen: hij versleutelt ze met een sleutel die alleen de apparaten van de host hebben, en bewaart alleen die verzegelde kopie. Een later datalek, een back-up of een inbraak achteraf maakt ze dus niet zichtbaar. Maar op het moment van boeken gaan de gegevens leesbaar door de server. De beheerder, of iemand die op dat moment heeft ingebroken, zou ze dan kunnen lezen. Naast de verzegelde kopie bewaart de server de tijd, de soort afspraak en de status, hoe er is geboekt (via de webpagina, de JSON-API of MCP), een hash van de sleutel in de beheerlink van de gast en, zolang de afspraak loopt, een hash van het e-mailadres van de gast, gekoppeld aan die tijd, zodat dezelfde afspraak niet twee keer wordt gemaakt.',
      'Een afspraak blijft op deze server tot 30 dagen na de afspraak. Annuleert de gast, weigert de host of verloopt een verzoek, dan worden de verzegelde gegevens en de hash van het e-mailadres meteen verwijderd, en de rest 30 dagen later. Zet de host de pagina uit of verwijdert de host het account, dan worden de pagina en alle afspraken erop verwijderd.',
      'Een gast kan de afspraak zelf annuleren met de link op de pagina die na het boeken verscheen; daarmee verdwijnen de gegevens meteen van deze server. Voor wat de host met je gegevens doet, neem je contact op met de host. Voor de kopie op deze server neem je contact met ons op.',
      'Hosts schrijven hun eigen pagina; wij controleren die niet. Doet een pagina zich voor als iemand anders, of is hij kwetsend? Gebruik dan Pagina melden onderaan de pagina, of neem contact met ons op.'
    ],
    security: 'Beveiliging',
    securityText: (protocol) =>
      `Je wachtwoord gaat op je apparaat door Argon2id, en je records worden met AES-256-GCM versleuteld voordat ze het verlaten. We kunnen je wachtwoord niet resetten: zonder je wachtwoord of je herstelsleutel kan niemand je gegevens openen, wij ook niet. ${protocol} beschrijft precies hoe het werkt.`,
    protocol: 'Het Rukoo-accountprotocol',
    changes: 'Wijzigingen',
    changesText: 'Als deze verklaring verandert op een manier die voor jou uitmaakt, verandert de datum bovenaan ook.'
  }
};

// The language asked for with ?lang=, or else the browser's first choice of English and Dutch.
export function privacyLanguage(url, acceptLanguage) {
  const asked = url.searchParams.get('lang');
  if (asked === 'nl' || asked === 'en') return asked;
  for (const part of String(acceptLanguage || '').split(',')) {
    const tag = part.trim().slice(0, 2).toLowerCase();
    if (tag === 'nl' || tag === 'en') return tag;
  }
  return 'en';
}

function contactLink(value) {
  if (!value) return '';
  return /^[^\s@]+@[^\s@]+$/.test(value) ? `<a href="mailto:${escape(value)}">${escape(value)}</a>` : link(value, value);
}

export function privacy({ operator, jurisdiction }, lang = 'en') {
  const T = TEXT[lang] || TEXT.en;
  const who = operator.name ? T.whoText(link(operator.url, operator.name)) : escape(T.unnamed);
  const contact = operator.contact ? T.contact(contactLink(operator.contact)) : '';
  const place = T.places[jurisdiction] || T.places.any;
  return `<!doctype html>
<html lang="${lang}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escape(T.title)}</title>
<style>
body { font: 16px/1.55 system-ui, sans-serif; max-width: 40rem; margin: 2.5rem auto; padding: 0 1.25rem; color: #1f2328; }
h1 { font-size: 1.5rem; } h2 { font-size: 1.1rem; margin-top: 2rem; } a { color: #0b62c4; } li { margin: 0.35rem 0; }
@media (prefers-color-scheme: dark) { body { background: #16181c; color: #e6e8eb; } a { color: #6cb0ff; } }
</style>
</head>
<body>
<h1>${escape(T.title)}</h1>
<p>${escape(T.updated(UPDATED))} ${T.other}</p>

<h2>${escape(T.who)}</h2>
<p>${who}${contact}</p>

<h2>${escape(T.what)}</h2>
<p>${escape(T.whatText)}</p>

<h2>${escape(T.data)}</h2>
<ul>
${T.dataItems.map((item) => `<li>${escape(item)}</li>`).join('\n')}
</ul>
<p>${escape(T.ip)}</p>

<h2>${escape(T.why)}</h2>
<p>${escape(T.whyText)}</p>

<h2>${escape(T.where)}</h2>
<p>${escape(T.whereText(place))}</p>

<h2>${escape(T.keep)}</h2>
<p>${escape(T.keepText)}</p>

<h2>${escape(T.rights)}</h2>
<p>${escape(T.rightsText)}</p>

<h2 id="booking-pages">${escape(T.booking)}</h2>
${T.bookingText.map((p) => `<p>${escape(p)}</p>`).join('\n')}

<h2>${escape(T.security)}</h2>
<p>${T.securityText(link(PROTOCOL_URL, T.protocol))}</p>

<h2>${escape(T.changes)}</h2>
<p>${escape(T.changesText)}</p>
</body>
</html>
`;
}
