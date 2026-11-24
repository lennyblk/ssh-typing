//! Textes de course. ASCII uniquement : tapables sur n'importe quel clavier.

pub const TEXTS: &[&str] = &[
    "la cendre tombe sans bruit sur les remparts. personne ne se souvient du nom des rois, mais tous se souviennent du froid.",
    "celui qui court plus vite que la nuit n'a pas besoin de lanterne. garde le souffle court, la main ferme et le regard loin devant.",
    "les marches de la crypte sont usees par ceux qui descendent. aucune trace ne remonte, et pourtant quelqu'un chante en bas.",
    "un forgeron sans feu reste un forgeron. il attend, il ecoute le vent, et quand l'etincelle revient il frappe avant tout le monde.",
    "ne compte pas les coups, compte les respirations. la lame la plus rapide appartient toujours a celui qui ne tremble pas.",
    "au bout du marais se dresse une tour sans porte. les voyageurs frappent aux murs, et parfois les murs repondent.",
    "trois cloches pour les vivants, deux pour les morts, une seule pour ceux qui ont oublie de quel cote ils se trouvent.",
    "la pluie efface les pas mais jamais la route. marche droit, tombe souvent, releve-toi plus vite que ton ombre.",
    "le givre a pris la plaine en une nuit. au matin, seules les corneilles savaient encore ou passait la riviere.",
    "on raconte que le dernier garde dort debout, la main sur la poignee. on raconte aussi qu'il ne dort pas vraiment.",
    "chaque lettre est un pas, chaque mot une foulee. la ligne d'arrivee n'attend personne et la cendre recouvre les derniers.",
    "sous la montagne, le feu parle une langue ancienne. les mineurs l'ecoutent, hochent la tete, et creusent ailleurs.",
];

pub fn pick(previous: Option<usize>) -> usize {
    loop {
        let i = rand::random_range(0..TEXTS.len());
        if Some(i) != previous {
            return i;
        }
    }
}
