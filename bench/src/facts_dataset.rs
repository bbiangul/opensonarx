use crate::facts_metrics::{EntitySpan, RelationSpan};

/// A benchmark sample with annotated ground truth.
pub struct FactsBenchmarkSample {
    pub id: String,
    pub language: String,
    pub category: String,
    pub text: String,
    pub expected_entities: Vec<ExpectedEntity>,
    pub expected_facts: Vec<ExpectedFact>,
}

pub struct ExpectedEntity {
    pub mention: String,
    pub entity_type: String,
    pub start: usize,
    pub end: usize,
}

pub struct ExpectedFact {
    pub subject: String,
    pub predicate: String,
    pub object: String,
}

impl ExpectedEntity {
    pub fn to_span(&self) -> EntitySpan {
        EntitySpan {
            start: self.start,
            end: self.end,
            entity_type: self.entity_type.clone(),
        }
    }
}

impl ExpectedFact {
    pub fn to_rel(&self) -> RelationSpan {
        RelationSpan {
            subject: self.subject.clone(),
            predicate: self.predicate.clone(),
            object: self.object.clone(),
        }
    }
}

/// Create an expected entity. The start/end offsets are placeholders —
/// they get recomputed from the text by `fix_offsets()` below.
fn e(mention: &str, entity_type: &str, _start: usize, _end: usize) -> ExpectedEntity {
    ExpectedEntity {
        mention: mention.to_string(),
        entity_type: entity_type.to_string(),
        start: 0,
        end: 0,
    }
}

/// Recompute byte offsets for all expected entities by finding each mention
/// in the sample text. Searches left-to-right, consuming positions so that
/// duplicate mentions get distinct offsets.
fn fix_offsets(samples: &mut [FactsBenchmarkSample]) {
    for sample in samples.iter_mut() {
        let text = &sample.text;
        let mut search_from = 0usize;
        for entity in &mut sample.expected_entities {
            if let Some(pos) = text[search_from..].find(&entity.mention) {
                let start = search_from + pos;
                entity.start = start;
                entity.end = start + entity.mention.len();
                // Advance past this match to handle duplicates
                search_from = entity.end;
            } else {
                // Fall back: search from beginning
                if let Some(pos) = text.find(&entity.mention) {
                    entity.start = pos;
                    entity.end = pos + entity.mention.len();
                }
            }
        }
    }
}

fn f(subject: &str, predicate: &str, object: &str) -> ExpectedFact {
    ExpectedFact {
        subject: subject.to_string(),
        predicate: predicate.to_string(),
        object: object.to_string(),
    }
}

/// Load all curated multilingual benchmark samples.
pub fn load_dataset() -> Vec<FactsBenchmarkSample> {
    let mut samples = Vec::new();
    samples.extend(english_samples());
    samples.extend(portuguese_samples());
    samples.extend(spanish_samples());
    samples.extend(german_samples());
    samples.extend(french_samples());
    samples.extend(chinese_samples());
    samples.extend(japanese_samples());
    fix_offsets(&mut samples);
    samples
}

fn english_samples() -> Vec<FactsBenchmarkSample> {
    vec![
        FactsBenchmarkSample {
            id: "en-wiki-01".into(),
            language: "en".into(),
            category: "wiki".into(),
            text: "Microsoft Corporation was founded by Bill Gates and Paul Allen on April 4, 1975, in Albuquerque, New Mexico.".into(),
            expected_entities: vec![
                e("Microsoft Corporation", "organization", 0, 21),
                e("Bill Gates", "person", 37, 47),
                e("Paul Allen", "person", 52, 62),
                e("April 4, 1975", "date", 66, 79),
                e("Albuquerque", "location", 84, 95),
                e("New Mexico", "location", 97, 107),
            ],
            expected_facts: vec![
                f("microsoft", "founded_by", "bill gates"),
                f("microsoft", "founded_by", "paul allen"),
                f("microsoft", "located_in", "albuquerque"),
            ],
        },
        FactsBenchmarkSample {
            id: "en-news-01".into(),
            language: "en".into(),
            category: "news".into(),
            text: "Tesla CEO Elon Musk announced a new Gigafactory in Austin, Texas on March 15, 2026.".into(),
            expected_entities: vec![
                e("Tesla", "organization", 0, 5),
                e("Elon Musk", "person", 10, 19),
                e("Gigafactory", "product", 36, 47),
                e("Austin", "location", 51, 57),
                e("Texas", "location", 59, 64),
                e("March 15, 2026", "date", 68, 82),
            ],
            expected_facts: vec![
                f("elon musk", "ceo_of", "tesla"),
                f("gigafactory", "located_in", "austin"),
            ],
        },
        FactsBenchmarkSample {
            id: "en-sports-01".into(),
            language: "en".into(),
            category: "sports".into(),
            text: "LeBron James scored 38 points as the Los Angeles Lakers defeated the Boston Celtics 112-108 at Staples Center on January 7, 2026.".into(),
            expected_entities: vec![
                e("LeBron James", "person", 0, 12),
                e("Los Angeles Lakers", "organization", 36, 54),
                e("Boston Celtics", "organization", 68, 82),
                e("Staples Center", "location", 94, 108),
                e("January 7, 2026", "date", 112, 127),
            ],
            expected_facts: vec![
                f("lebron james", "plays_for", "los angeles lakers"),
            ],
        },
        FactsBenchmarkSample {
            id: "en-tech-01".into(),
            language: "en".into(),
            category: "tech".into(),
            text: "OpenAI released GPT-5 in San Francisco, built on a 10 trillion parameter architecture designed by Ilya Sutskever.".into(),
            expected_entities: vec![
                e("OpenAI", "organization", 0, 6),
                e("GPT-5", "product", 16, 21),
                e("San Francisco", "location", 25, 38),
                e("Ilya Sutskever", "person", 96, 110),
            ],
            expected_facts: vec![
                f("openai", "released", "gpt-5"),
            ],
        },
        FactsBenchmarkSample {
            id: "en-company-01".into(),
            language: "en".into(),
            category: "company".into(),
            text: "Amazon was founded by Jeff Bezos in 1994 in Bellevue, Washington. Andy Jassy became CEO in July 2021.".into(),
            expected_entities: vec![
                e("Amazon", "organization", 0, 6),
                e("Jeff Bezos", "person", 22, 32),
                e("Bellevue", "location", 44, 52),
                e("Washington", "location", 54, 64),
                e("Andy Jassy", "person", 66, 76),
            ],
            expected_facts: vec![
                f("amazon", "founded_by", "jeff bezos"),
                f("andy jassy", "ceo_of", "amazon"),
            ],
        },
        FactsBenchmarkSample {
            id: "en-wiki-02".into(),
            language: "en".into(),
            category: "wiki".into(),
            text: "The Eiffel Tower is a wrought-iron lattice tower in Paris, France. It was designed by Gustave Eiffel and completed in 1889.".into(),
            expected_entities: vec![
                e("Eiffel Tower", "location", 4, 16),
                e("Paris", "location", 49, 54),
                e("France", "location", 56, 62),
                e("Gustave Eiffel", "person", 85, 99),
            ],
            expected_facts: vec![
                f("eiffel tower", "located_in", "paris"),
                f("eiffel tower", "designed_by", "gustave eiffel"),
            ],
        },
        FactsBenchmarkSample {
            id: "en-news-02".into(),
            language: "en".into(),
            category: "news".into(),
            text: "The United Nations General Assembly convened in New York on September 20, 2025, with Secretary-General Antonio Guterres calling for climate action.".into(),
            expected_entities: vec![
                e("United Nations General Assembly", "organization", 4, 35),
                e("New York", "location", 49, 57),
                e("September 20, 2025", "date", 61, 79),
                e("Antonio Guterres", "person", 107, 123),
            ],
            expected_facts: vec![
                f("antonio guterres", "member_of", "united nations"),
            ],
        },
        FactsBenchmarkSample {
            id: "en-sports-02".into(),
            language: "en".into(),
            category: "sports".into(),
            text: "Lionel Messi led Inter Miami to the MLS Cup Final against LAFC at BMO Stadium on December 7, 2025.".into(),
            expected_entities: vec![
                e("Lionel Messi", "person", 0, 12),
                e("Inter Miami", "organization", 17, 28),
                e("MLS Cup Final", "event", 36, 49),
                e("LAFC", "organization", 58, 62),
                e("BMO Stadium", "location", 66, 77),
                e("December 7, 2025", "date", 81, 97),
            ],
            expected_facts: vec![
                f("lionel messi", "plays_for", "inter miami"),
            ],
        },
        FactsBenchmarkSample {
            id: "en-tech-02".into(),
            language: "en".into(),
            category: "tech".into(),
            text: "Apple unveiled the Vision Pro 2 at WWDC 2026 in Cupertino, California, priced at $2,999.".into(),
            expected_entities: vec![
                e("Apple", "organization", 0, 5),
                e("Vision Pro 2", "product", 18, 30),
                e("WWDC 2026", "event", 34, 43),
                e("Cupertino", "location", 47, 56),
                e("California", "location", 58, 68),
            ],
            expected_facts: vec![
                f("apple", "released", "vision pro 2"),
            ],
        },
        FactsBenchmarkSample {
            id: "en-company-02".into(),
            language: "en".into(),
            category: "company".into(),
            text: "Alphabet Inc is the parent company of Google, headquartered in Mountain View, California. Sundar Pichai serves as CEO.".into(),
            expected_entities: vec![
                e("Alphabet Inc", "organization", 0, 12),
                e("Google", "organization", 40, 46),
                e("Mountain View", "location", 65, 78),
                e("California", "location", 80, 90),
                e("Sundar Pichai", "person", 92, 105),
            ],
            expected_facts: vec![
                f("google", "part_of", "alphabet inc"),
                f("sundar pichai", "ceo_of", "alphabet inc"),
            ],
        },
    ]
}

fn portuguese_samples() -> Vec<FactsBenchmarkSample> {
    vec![
        FactsBenchmarkSample {
            id: "pt-sports-01".into(),
            language: "pt".into(),
            category: "sports".into(),
            text: "O Flamengo venceu o Palmeiras por 2 a 1 no Maracanã, com gols de Pedro e Arrascaeta.".into(),
            expected_entities: vec![
                e("Flamengo", "organization", 2, 10),
                e("Palmeiras", "organization", 20, 29),
                e("Maracanã", "location", 44, 52),
                e("Pedro", "person", 67, 72),
                e("Arrascaeta", "person", 75, 85),
            ],
            expected_facts: vec![
                f("pedro", "plays_for", "flamengo"),
                f("arrascaeta", "plays_for", "flamengo"),
            ],
        },
        FactsBenchmarkSample {
            id: "pt-news-01".into(),
            language: "pt".into(),
            category: "news".into(),
            text: "O presidente Lula anunciou novos investimentos de R$ 50 bilhões para infraestrutura em São Paulo e Rio de Janeiro.".into(),
            expected_entities: vec![
                e("Lula", "person", 15, 19),
                e("São Paulo", "location", 88, 97),
                e("Rio de Janeiro", "location", 100, 114),
            ],
            expected_facts: vec![],
        },
        FactsBenchmarkSample {
            id: "pt-wiki-01".into(),
            language: "pt".into(),
            category: "wiki".into(),
            text: "O Cristo Redentor é uma estátua de Jesus Cristo localizada no topo do morro do Corcovado, na cidade do Rio de Janeiro, no Brasil.".into(),
            expected_entities: vec![
                e("Cristo Redentor", "location", 2, 17),
                e("Jesus Cristo", "person", 37, 49),
                e("Corcovado", "location", 80, 89),
                e("Rio de Janeiro", "location", 103, 117),
                e("Brasil", "location", 122, 128),
            ],
            expected_facts: vec![
                f("cristo redentor", "located_in", "rio de janeiro"),
            ],
        },
        FactsBenchmarkSample {
            id: "pt-tech-01".into(),
            language: "pt".into(),
            category: "tech".into(),
            text: "A Embraer entregou o primeiro jato E195-E2 para a Azul Linhas Aéreas em São José dos Campos.".into(),
            expected_entities: vec![
                e("Embraer", "organization", 2, 9),
                e("E195-E2", "product", 33, 40),
                e("Azul Linhas Aéreas", "organization", 47, 65),
                e("São José dos Campos", "location", 69, 88),
            ],
            expected_facts: vec![
                f("embraer", "located_in", "são josé dos campos"),
            ],
        },
        FactsBenchmarkSample {
            id: "pt-news-02".into(),
            language: "pt".into(),
            category: "news".into(),
            text: "Tarcísio de Freitas, governador de São Paulo, inaugurou a nova linha do Metrô na capital paulista.".into(),
            expected_entities: vec![
                e("Tarcísio de Freitas", "person", 0, 19),
                e("São Paulo", "location", 35, 44),
            ],
            expected_facts: vec![],
        },
        FactsBenchmarkSample {
            id: "pt-sports-02".into(),
            language: "pt".into(),
            category: "sports".into(),
            text: "Vini Jr marcou dois gols pelo Real Madrid na vitória contra o Barcelona no Santiago Bernabéu.".into(),
            expected_entities: vec![
                e("Vini Jr", "person", 0, 7),
                e("Real Madrid", "organization", 30, 41),
                e("Barcelona", "organization", 62, 71),
                e("Santiago Bernabéu", "location", 75, 92),
            ],
            expected_facts: vec![
                f("vini jr", "plays_for", "real madrid"),
            ],
        },
        FactsBenchmarkSample {
            id: "pt-wiki-02".into(),
            language: "pt".into(),
            category: "wiki".into(),
            text: "A Universidade de São Paulo foi fundada em 1934 e é a maior universidade do Brasil.".into(),
            expected_entities: vec![
                e("Universidade de São Paulo", "organization", 2, 27),
                e("Brasil", "location", 76, 82),
            ],
            expected_facts: vec![
                f("universidade de são paulo", "located_in", "brasil"),
            ],
        },
        FactsBenchmarkSample {
            id: "pt-company-01".into(),
            language: "pt".into(),
            category: "company".into(),
            text: "A Petrobras, com sede no Rio de Janeiro, é a maior empresa estatal do Brasil, presidida por Jean Paul Prates.".into(),
            expected_entities: vec![
                e("Petrobras", "organization", 2, 11),
                e("Rio de Janeiro", "location", 24, 38),
                e("Brasil", "location", 75, 81),
                e("Jean Paul Prates", "person", 96, 112),
            ],
            expected_facts: vec![
                f("petrobras", "located_in", "rio de janeiro"),
                f("jean paul prates", "ceo_of", "petrobras"),
            ],
        },
        FactsBenchmarkSample {
            id: "pt-tech-02".into(),
            language: "pt".into(),
            category: "tech".into(),
            text: "O Nubank, fundado por David Vélez em São Paulo, atingiu 100 milhões de clientes na América Latina.".into(),
            expected_entities: vec![
                e("Nubank", "organization", 2, 8),
                e("David Vélez", "person", 23, 34),
                e("São Paulo", "location", 38, 47),
                e("América Latina", "location", 84, 98),
            ],
            expected_facts: vec![
                f("nubank", "founded_by", "david vélez"),
            ],
        },
        FactsBenchmarkSample {
            id: "pt-news-03".into(),
            language: "pt".into(),
            category: "news".into(),
            text: "O Banco Central do Brasil, sob a presidência de Roberto Campos Neto, manteve a taxa Selic em 13,75% ao ano.".into(),
            expected_entities: vec![
                e("Banco Central do Brasil", "organization", 2, 25),
                e("Roberto Campos Neto", "person", 48, 67),
            ],
            expected_facts: vec![
                f("roberto campos neto", "ceo_of", "banco central do brasil"),
            ],
        },
    ]
}

fn spanish_samples() -> Vec<FactsBenchmarkSample> {
    vec![
        FactsBenchmarkSample {
            id: "es-news-01".into(),
            language: "es".into(),
            category: "news".into(),
            text: "El presidente Pedro Sánchez visitó la sede de la OTAN en Bruselas para reunirse con Jens Stoltenberg.".into(),
            expected_entities: vec![
                e("Pedro Sánchez", "person", 15, 28),
                e("OTAN", "organization", 50, 54),
                e("Bruselas", "location", 58, 66),
                e("Jens Stoltenberg", "person", 85, 101),
            ],
            expected_facts: vec![],
        },
        FactsBenchmarkSample {
            id: "es-wiki-01".into(),
            language: "es".into(),
            category: "wiki".into(),
            text: "La Sagrada Familia es una basílica en Barcelona, diseñada por Antoni Gaudí. Su construcción comenzó en 1882.".into(),
            expected_entities: vec![
                e("Sagrada Familia", "location", 3, 18),
                e("Barcelona", "location", 38, 47),
                e("Antoni Gaudí", "person", 63, 75),
            ],
            expected_facts: vec![
                f("sagrada familia", "located_in", "barcelona"),
                f("sagrada familia", "designed_by", "antoni gaudí"),
            ],
        },
        FactsBenchmarkSample {
            id: "es-sports-01".into(),
            language: "es".into(),
            category: "sports".into(),
            text: "El Real Madrid ganó la Liga de Campeones en el Estadio de Wembley con un gol de Vinícius Júnior.".into(),
            expected_entities: vec![
                e("Real Madrid", "organization", 3, 14),
                e("Liga de Campeones", "event", 22, 39),
                e("Estadio de Wembley", "location", 46, 64),
                e("Vinícius Júnior", "person", 79, 94),
            ],
            expected_facts: vec![
                f("vinícius júnior", "plays_for", "real madrid"),
            ],
        },
        FactsBenchmarkSample {
            id: "es-company-01".into(),
            language: "es".into(),
            category: "company".into(),
            text: "Inditex, con sede en Arteixo, Galicia, es la empresa matriz de Zara. Su fundador es Amancio Ortega.".into(),
            expected_entities: vec![
                e("Inditex", "organization", 0, 7),
                e("Arteixo", "location", 20, 27),
                e("Galicia", "location", 29, 36),
                e("Zara", "organization", 63, 67),
                e("Amancio Ortega", "person", 84, 98),
            ],
            expected_facts: vec![
                f("zara", "part_of", "inditex"),
                f("inditex", "founded_by", "amancio ortega"),
            ],
        },
        FactsBenchmarkSample {
            id: "es-news-02".into(),
            language: "es".into(),
            category: "news".into(),
            text: "Felipe VI inauguró la nueva terminal del aeropuerto de Madrid-Barajas junto con la ministra de Transportes.".into(),
            expected_entities: vec![
                e("Felipe VI", "person", 0, 9),
                e("Madrid-Barajas", "location", 56, 70),
            ],
            expected_facts: vec![],
        },
        FactsBenchmarkSample {
            id: "es-wiki-02".into(),
            language: "es".into(),
            category: "wiki".into(),
            text: "Machu Picchu es una ciudadela inca ubicada en la cordillera de los Andes, en la región de Cusco, Perú.".into(),
            expected_entities: vec![
                e("Machu Picchu", "location", 0, 12),
                e("Andes", "location", 66, 71),
                e("Cusco", "location", 90, 95),
                e("Perú", "location", 97, 101),
            ],
            expected_facts: vec![
                f("machu picchu", "located_in", "cusco"),
            ],
        },
        FactsBenchmarkSample {
            id: "es-tech-01".into(),
            language: "es".into(),
            category: "tech".into(),
            text: "Mercado Libre, fundado por Marcos Galperín en Buenos Aires, es la empresa tecnológica más grande de América Latina.".into(),
            expected_entities: vec![
                e("Mercado Libre", "organization", 0, 13),
                e("Marcos Galperín", "person", 27, 42),
                e("Buenos Aires", "location", 46, 58),
                e("América Latina", "location", 100, 114),
            ],
            expected_facts: vec![
                f("mercado libre", "founded_by", "marcos galperín"),
                f("mercado libre", "located_in", "buenos aires"),
            ],
        },
        FactsBenchmarkSample {
            id: "es-sports-02".into(),
            language: "es".into(),
            category: "sports".into(),
            text: "Aitana Bonmatí ganó el Balón de Oro femenino en una ceremonia en París organizada por France Football.".into(),
            expected_entities: vec![
                e("Aitana Bonmatí", "person", 0, 14),
                e("Balón de Oro", "event", 22, 34),
                e("París", "location", 65, 70),
                e("France Football", "organization", 86, 101),
            ],
            expected_facts: vec![],
        },
        FactsBenchmarkSample {
            id: "es-company-02".into(),
            language: "es".into(),
            category: "company".into(),
            text: "BBVA, con sede en Bilbao, España, anunció una inversión de 2.000 millones de euros en inteligencia artificial.".into(),
            expected_entities: vec![
                e("BBVA", "organization", 0, 4),
                e("Bilbao", "location", 17, 23),
                e("España", "location", 25, 31),
            ],
            expected_facts: vec![
                f("bbva", "located_in", "bilbao"),
            ],
        },
        FactsBenchmarkSample {
            id: "es-news-03".into(),
            language: "es".into(),
            category: "news".into(),
            text: "La selección de Argentina, dirigida por Lionel Scaloni, ganó la Copa América en Miami.".into(),
            expected_entities: vec![
                e("Argentina", "location", 18, 27),
                e("Lionel Scaloni", "person", 42, 56),
                e("Copa América", "event", 65, 77),
                e("Miami", "location", 81, 86),
            ],
            expected_facts: vec![],
        },
    ]
}

fn german_samples() -> Vec<FactsBenchmarkSample> {
    vec![
        FactsBenchmarkSample {
            id: "de-news-01".into(),
            language: "de".into(),
            category: "news".into(),
            text: "Bundeskanzler Olaf Scholz traf sich mit Emmanuel Macron in Berlin, um über die europäische Verteidigungspolitik zu sprechen.".into(),
            expected_entities: vec![
                e("Olaf Scholz", "person", 14, 25),
                e("Emmanuel Macron", "person", 39, 54),
                e("Berlin", "location", 58, 64),
            ],
            expected_facts: vec![],
        },
        FactsBenchmarkSample {
            id: "de-wiki-01".into(),
            language: "de".into(),
            category: "wiki".into(),
            text: "Das Brandenburger Tor ist ein Wahrzeichen Berlins und wurde 1791 fertiggestellt. Es befindet sich am Pariser Platz.".into(),
            expected_entities: vec![
                e("Brandenburger Tor", "location", 4, 21),
                e("Berlins", "location", 42, 49),
                e("Pariser Platz", "location", 101, 114),
            ],
            expected_facts: vec![
                f("brandenburger tor", "located_in", "berlin"),
            ],
        },
        FactsBenchmarkSample {
            id: "de-sports-01".into(),
            language: "de".into(),
            category: "sports".into(),
            text: "Bayern München besiegte Borussia Dortmund 3:1 in der Allianz Arena. Jamal Musiala erzielte zwei Tore.".into(),
            expected_entities: vec![
                e("Bayern München", "organization", 0, 14),
                e("Borussia Dortmund", "organization", 24, 41),
                e("Allianz Arena", "location", 52, 65),
                e("Jamal Musiala", "person", 67, 80),
            ],
            expected_facts: vec![
                f("jamal musiala", "plays_for", "bayern münchen"),
            ],
        },
        FactsBenchmarkSample {
            id: "de-company-01".into(),
            language: "de".into(),
            category: "company".into(),
            text: "Volkswagen, mit Hauptsitz in Wolfsburg, ist der größte Automobilhersteller Europas. Oliver Blume ist Vorstandsvorsitzender.".into(),
            expected_entities: vec![
                e("Volkswagen", "organization", 0, 10),
                e("Wolfsburg", "location", 29, 38),
                e("Europas", "location", 72, 79),
                e("Oliver Blume", "person", 81, 92),
            ],
            expected_facts: vec![
                f("volkswagen", "located_in", "wolfsburg"),
                f("oliver blume", "ceo_of", "volkswagen"),
            ],
        },
        FactsBenchmarkSample {
            id: "de-tech-01".into(),
            language: "de".into(),
            category: "tech".into(),
            text: "SAP, gegründet 1972 in Walldorf von Hasso Plattner, ist Europas größtes Softwareunternehmen.".into(),
            expected_entities: vec![
                e("SAP", "organization", 0, 3),
                e("Walldorf", "location", 22, 30),
                e("Hasso Plattner", "person", 35, 49),
            ],
            expected_facts: vec![
                f("sap", "founded_by", "hasso plattner"),
                f("sap", "located_in", "walldorf"),
            ],
        },
        FactsBenchmarkSample {
            id: "de-wiki-02".into(),
            language: "de".into(),
            category: "wiki".into(),
            text: "Die Zugspitze ist mit 2.962 Metern der höchste Berg Deutschlands und liegt an der Grenze zwischen Bayern und Tirol.".into(),
            expected_entities: vec![
                e("Zugspitze", "location", 4, 13),
                e("Deutschlands", "location", 52, 64),
                e("Bayern", "location", 96, 102),
                e("Tirol", "location", 107, 112),
            ],
            expected_facts: vec![
                f("zugspitze", "located_in", "deutschland"),
            ],
        },
        FactsBenchmarkSample {
            id: "de-news-02".into(),
            language: "de".into(),
            category: "news".into(),
            text: "Annalena Baerbock reiste nach Kiew, um die Unterstützung Deutschlands für die Ukraine zu bekräftigen.".into(),
            expected_entities: vec![
                e("Annalena Baerbock", "person", 0, 17),
                e("Kiew", "location", 29, 33),
                e("Deutschlands", "location", 54, 66),
                e("Ukraine", "location", 75, 82),
            ],
            expected_facts: vec![],
        },
        FactsBenchmarkSample {
            id: "de-sports-02".into(),
            language: "de".into(),
            category: "sports".into(),
            text: "Max Verstappen gewann den Großen Preis von Monza für Red Bull Racing und sicherte sich die WM-Führung.".into(),
            expected_entities: vec![
                e("Max Verstappen", "person", 0, 14),
                e("Monza", "location", 43, 48),
                e("Red Bull Racing", "organization", 53, 68),
            ],
            expected_facts: vec![
                f("max verstappen", "plays_for", "red bull racing"),
            ],
        },
        FactsBenchmarkSample {
            id: "de-company-02".into(),
            language: "de".into(),
            category: "company".into(),
            text: "Siemens, gegründet von Werner von Siemens in Berlin, ist ein globaler Technologiekonzern mit Sitz in München.".into(),
            expected_entities: vec![
                e("Siemens", "organization", 0, 7),
                e("Werner von Siemens", "person", 23, 41),
                e("Berlin", "location", 45, 51),
                e("München", "location", 100, 107),
            ],
            expected_facts: vec![
                f("siemens", "founded_by", "werner von siemens"),
                f("siemens", "located_in", "münchen"),
            ],
        },
        FactsBenchmarkSample {
            id: "de-tech-02".into(),
            language: "de".into(),
            category: "tech".into(),
            text: "BioNTech, gegründet von Ugur Sahin und Özlem Türeci in Mainz, entwickelte den ersten mRNA-Impfstoff gegen COVID-19.".into(),
            expected_entities: vec![
                e("BioNTech", "organization", 0, 8),
                e("Ugur Sahin", "person", 24, 34),
                e("Özlem Türeci", "person", 39, 51),
                e("Mainz", "location", 55, 60),
            ],
            expected_facts: vec![
                f("biontech", "founded_by", "ugur sahin"),
                f("biontech", "located_in", "mainz"),
            ],
        },
    ]
}

fn french_samples() -> Vec<FactsBenchmarkSample> {
    vec![
        FactsBenchmarkSample {
            id: "fr-news-01".into(),
            language: "fr".into(),
            category: "news".into(),
            text: "Le président Emmanuel Macron a reçu Angela Merkel à l'Élysée pour discuter de la politique européenne.".into(),
            expected_entities: vec![
                e("Emmanuel Macron", "person", 14, 29),
                e("Angela Merkel", "person", 39, 52),
                e("Élysée", "location", 58, 64),
            ],
            expected_facts: vec![],
        },
        FactsBenchmarkSample {
            id: "fr-wiki-01".into(),
            language: "fr".into(),
            category: "wiki".into(),
            text: "La Tour Eiffel a été construite par Gustave Eiffel pour l'Exposition universelle de 1889 à Paris.".into(),
            expected_entities: vec![
                e("Tour Eiffel", "location", 3, 14),
                e("Gustave Eiffel", "person", 35, 49),
                e("Paris", "location", 91, 96),
            ],
            expected_facts: vec![
                f("tour eiffel", "designed_by", "gustave eiffel"),
                f("tour eiffel", "located_in", "paris"),
            ],
        },
        FactsBenchmarkSample {
            id: "fr-sports-01".into(),
            language: "fr".into(),
            category: "sports".into(),
            text: "Kylian Mbappé a marqué un triplé pour le Paris Saint-Germain au Parc des Princes contre l'Olympique de Marseille.".into(),
            expected_entities: vec![
                e("Kylian Mbappé", "person", 0, 13),
                e("Paris Saint-Germain", "organization", 40, 59),
                e("Parc des Princes", "location", 63, 79),
                e("Olympique de Marseille", "organization", 90, 112),
            ],
            expected_facts: vec![
                f("kylian mbappé", "plays_for", "paris saint-germain"),
            ],
        },
        FactsBenchmarkSample {
            id: "fr-company-01".into(),
            language: "fr".into(),
            category: "company".into(),
            text: "LVMH, dirigé par Bernard Arnault, est basé à Paris et possède Louis Vuitton, Dior et Moët Hennessy.".into(),
            expected_entities: vec![
                e("LVMH", "organization", 0, 4),
                e("Bernard Arnault", "person", 17, 32),
                e("Paris", "location", 45, 50),
                e("Louis Vuitton", "organization", 61, 74),
                e("Dior", "organization", 76, 80),
                e("Moët Hennessy", "organization", 84, 97),
            ],
            expected_facts: vec![
                f("bernard arnault", "ceo_of", "lvmh"),
                f("louis vuitton", "part_of", "lvmh"),
            ],
        },
        FactsBenchmarkSample {
            id: "fr-tech-01".into(),
            language: "fr".into(),
            category: "tech".into(),
            text: "Dassault Systèmes, fondée par Charles Edelstenne à Vélizy-Villacoublay, développe le logiciel CATIA.".into(),
            expected_entities: vec![
                e("Dassault Systèmes", "organization", 0, 18),
                e("Charles Edelstenne", "person", 31, 49),
                e("Vélizy-Villacoublay", "location", 52, 71),
                e("CATIA", "product", 92, 97),
            ],
            expected_facts: vec![
                f("dassault systèmes", "founded_by", "charles edelstenne"),
            ],
        },
        FactsBenchmarkSample {
            id: "fr-wiki-02".into(),
            language: "fr".into(),
            category: "wiki".into(),
            text: "Le Louvre est le musée le plus visité au monde, situé à Paris. Il abrite la Joconde de Léonard de Vinci.".into(),
            expected_entities: vec![
                e("Louvre", "location", 3, 9),
                e("Paris", "location", 56, 61),
                e("Joconde", "product", 77, 84),
                e("Léonard de Vinci", "person", 88, 104),
            ],
            expected_facts: vec![
                f("louvre", "located_in", "paris"),
            ],
        },
        FactsBenchmarkSample {
            id: "fr-news-02".into(),
            language: "fr".into(),
            category: "news".into(),
            text: "Christine Lagarde, présidente de la BCE, a annoncé une hausse des taux d'intérêt à Francfort.".into(),
            expected_entities: vec![
                e("Christine Lagarde", "person", 0, 17),
                e("BCE", "organization", 37, 40),
                e("Francfort", "location", 82, 91),
            ],
            expected_facts: vec![
                f("christine lagarde", "ceo_of", "bce"),
            ],
        },
        FactsBenchmarkSample {
            id: "fr-sports-02".into(),
            language: "fr".into(),
            category: "sports".into(),
            text: "Teddy Riner a remporté sa troisième médaille d'or olympique en judo à Paris 2024.".into(),
            expected_entities: vec![
                e("Teddy Riner", "person", 0, 11),
                e("Paris", "location", 67, 72),
            ],
            expected_facts: vec![],
        },
        FactsBenchmarkSample {
            id: "fr-company-02".into(),
            language: "fr".into(),
            category: "company".into(),
            text: "TotalEnergies, dont le siège est à La Défense, est dirigée par Patrick Pouyanné. C'est le plus grand groupe énergétique français.".into(),
            expected_entities: vec![
                e("TotalEnergies", "organization", 0, 13),
                e("La Défense", "location", 34, 44),
                e("Patrick Pouyanné", "person", 62, 78),
            ],
            expected_facts: vec![
                f("totalenergies", "located_in", "la défense"),
                f("patrick pouyanné", "ceo_of", "totalenergies"),
            ],
        },
        FactsBenchmarkSample {
            id: "fr-tech-02".into(),
            language: "fr".into(),
            category: "tech".into(),
            text: "OVHcloud, fondée par Octave Klaba à Roubaix, est le plus grand hébergeur cloud européen.".into(),
            expected_entities: vec![
                e("OVHcloud", "organization", 0, 8),
                e("Octave Klaba", "person", 21, 33),
                e("Roubaix", "location", 36, 43),
            ],
            expected_facts: vec![
                f("ovhcloud", "founded_by", "octave klaba"),
                f("ovhcloud", "located_in", "roubaix"),
            ],
        },
    ]
}

fn chinese_samples() -> Vec<FactsBenchmarkSample> {
    vec![
        FactsBenchmarkSample {
            id: "zh-news-01".into(),
            language: "zh".into(),
            category: "news".into(),
            text: "习近平主席在北京会见了来访的法国总统马克龙，双方讨论了中法合作关系。".into(),
            expected_entities: vec![
                e("习近平", "person", 0, 9),
                e("北京", "location", 15, 21),
                e("法国", "location", 30, 36),
                e("马克龙", "person", 42, 51),
            ],
            expected_facts: vec![],
        },
        FactsBenchmarkSample {
            id: "zh-wiki-01".into(),
            language: "zh".into(),
            category: "wiki".into(),
            text: "长城是中国最著名的建筑之一，始建于公元前7世纪，横跨中国北部多个省份。".into(),
            expected_entities: vec![
                e("长城", "location", 0, 6),
                e("中国", "location", 9, 15),
            ],
            expected_facts: vec![
                f("长城", "located_in", "中国"),
            ],
        },
        FactsBenchmarkSample {
            id: "zh-sports-01".into(),
            language: "zh".into(),
            category: "sports".into(),
            text: "武磊在上海体育场为上海海港队攻入两球，帮助球队击败北京国安。".into(),
            expected_entities: vec![
                e("武磊", "person", 0, 6),
                e("上海体育场", "location", 9, 24),
                e("上海海港", "organization", 27, 39),
                e("北京国安", "organization", 54, 66),
            ],
            expected_facts: vec![
                f("武磊", "plays_for", "上海海港"),
            ],
        },
        FactsBenchmarkSample {
            id: "zh-company-01".into(),
            language: "zh".into(),
            category: "company".into(),
            text: "华为公司由任正非于1987年在深圳创立，是全球最大的电信设备制造商。".into(),
            expected_entities: vec![
                e("华为", "organization", 0, 6),
                e("任正非", "person", 12, 21),
                e("深圳", "location", 33, 39),
            ],
            expected_facts: vec![
                f("华为", "founded_by", "任正非"),
                f("华为", "located_in", "深圳"),
            ],
        },
        FactsBenchmarkSample {
            id: "zh-tech-01".into(),
            language: "zh".into(),
            category: "tech".into(),
            text: "腾讯公司总部位于深圳，由马化腾创立，旗下产品包括微信和QQ。".into(),
            expected_entities: vec![
                e("腾讯", "organization", 0, 6),
                e("深圳", "location", 15, 21),
                e("马化腾", "person", 24, 33),
                e("微信", "product", 48, 54),
            ],
            expected_facts: vec![
                f("腾讯", "founded_by", "马化腾"),
                f("腾讯", "located_in", "深圳"),
            ],
        },
        FactsBenchmarkSample {
            id: "zh-news-02".into(),
            language: "zh".into(),
            category: "news".into(),
            text: "上海市市长龚正宣布了新的自贸区扩展计划，将覆盖浦东新区更多区域。".into(),
            expected_entities: vec![
                e("上海", "location", 0, 6),
                e("龚正", "person", 15, 21),
                e("浦东新区", "location", 45, 57),
            ],
            expected_facts: vec![],
        },
        FactsBenchmarkSample {
            id: "zh-wiki-02".into(),
            language: "zh".into(),
            category: "wiki".into(),
            text: "故宫位于北京中心，是明清两代的皇家宫殿，建于1406年。".into(),
            expected_entities: vec![
                e("故宫", "location", 0, 6),
                e("北京", "location", 9, 15),
            ],
            expected_facts: vec![
                f("故宫", "located_in", "北京"),
            ],
        },
        FactsBenchmarkSample {
            id: "zh-sports-02".into(),
            language: "zh".into(),
            category: "sports".into(),
            text: "全红婵在东京奥运会跳水比赛中获得金牌，成为中国最年轻的奥运冠军之一。".into(),
            expected_entities: vec![
                e("全红婵", "person", 0, 9),
                e("东京", "location", 12, 18),
                e("中国", "location", 42, 48),
            ],
            expected_facts: vec![],
        },
        FactsBenchmarkSample {
            id: "zh-company-02".into(),
            language: "zh".into(),
            category: "company".into(),
            text: "阿里巴巴集团由马云于1999年在杭州创立，旗下拥有淘宝和天猫两大电商平台。".into(),
            expected_entities: vec![
                e("阿里巴巴", "organization", 0, 12),
                e("马云", "person", 15, 21),
                e("杭州", "location", 33, 39),
                e("淘宝", "organization", 48, 54),
                e("天猫", "organization", 57, 63),
            ],
            expected_facts: vec![
                f("阿里巴巴", "founded_by", "马云"),
                f("阿里巴巴", "located_in", "杭州"),
            ],
        },
        FactsBenchmarkSample {
            id: "zh-tech-02".into(),
            language: "zh".into(),
            category: "tech".into(),
            text: "字节跳动总部位于北京，由张一鸣创立，旗下产品包括抖音和TikTok。".into(),
            expected_entities: vec![
                e("字节跳动", "organization", 0, 12),
                e("北京", "location", 18, 24),
                e("张一鸣", "person", 27, 36),
            ],
            expected_facts: vec![
                f("字节跳动", "founded_by", "张一鸣"),
                f("字节跳动", "located_in", "北京"),
            ],
        },
    ]
}

fn japanese_samples() -> Vec<FactsBenchmarkSample> {
    vec![
        FactsBenchmarkSample {
            id: "ja-news-01".into(),
            language: "ja".into(),
            category: "news".into(),
            text: "岸田首相は東京で米国のバイデン大統領と会談し、日米同盟の強化について議論した。".into(),
            expected_entities: vec![
                e("岸田", "person", 0, 6),
                e("東京", "location", 12, 18),
                e("米国", "location", 21, 27),
                e("バイデン", "person", 30, 42),
            ],
            expected_facts: vec![],
        },
        FactsBenchmarkSample {
            id: "ja-wiki-01".into(),
            language: "ja".into(),
            category: "wiki".into(),
            text: "東京タワーは1958年に完成した電波塔で、東京都港区芝公園に位置している。".into(),
            expected_entities: vec![
                e("東京タワー", "location", 0, 15),
                e("東京都港区芝公園", "location", 33, 57),
            ],
            expected_facts: vec![
                f("東京タワー", "located_in", "東京都港区"),
            ],
        },
        FactsBenchmarkSample {
            id: "ja-sports-01".into(),
            language: "ja".into(),
            category: "sports".into(),
            text: "大谷翔平はロサンゼルス・ドジャースで46本塁打を記録し、MVPを受賞した。".into(),
            expected_entities: vec![
                e("大谷翔平", "person", 0, 12),
                e("ロサンゼルス・ドジャース", "organization", 15, 48),
            ],
            expected_facts: vec![
                f("大谷翔平", "plays_for", "ロサンゼルス・ドジャース"),
            ],
        },
        FactsBenchmarkSample {
            id: "ja-company-01".into(),
            language: "ja".into(),
            category: "company".into(),
            text: "トヨタ自動車は愛知県豊田市に本社を置く世界最大の自動車メーカーで、豊田章男が会長を務める。".into(),
            expected_entities: vec![
                e("トヨタ自動車", "organization", 0, 18),
                e("愛知県豊田市", "location", 21, 39),
                e("豊田章男", "person", 60, 72),
            ],
            expected_facts: vec![
                f("トヨタ自動車", "located_in", "愛知県豊田市"),
                f("豊田章男", "ceo_of", "トヨタ自動車"),
            ],
        },
        FactsBenchmarkSample {
            id: "ja-tech-01".into(),
            language: "ja".into(),
            category: "tech".into(),
            text: "ソフトバンクグループの孫正義会長は、東京で人工知能への1000億ドルの投資計画を発表した。".into(),
            expected_entities: vec![
                e("ソフトバンクグループ", "organization", 0, 30),
                e("孫正義", "person", 33, 42),
                e("東京", "location", 51, 57),
            ],
            expected_facts: vec![
                f("孫正義", "ceo_of", "ソフトバンクグループ"),
            ],
        },
        FactsBenchmarkSample {
            id: "ja-wiki-02".into(),
            language: "ja".into(),
            category: "wiki".into(),
            text: "富士山は静岡県と山梨県の境に位置し、標高3776メートルの日本最高峰である。".into(),
            expected_entities: vec![
                e("富士山", "location", 0, 9),
                e("静岡県", "location", 12, 21),
                e("山梨県", "location", 24, 33),
            ],
            expected_facts: vec![
                f("富士山", "located_in", "日本"),
            ],
        },
        FactsBenchmarkSample {
            id: "ja-news-02".into(),
            language: "ja".into(),
            category: "news".into(),
            text: "日本銀行の植田和男総裁はマイナス金利政策の解除を発表し、17年ぶりの利上げとなった。".into(),
            expected_entities: vec![
                e("日本銀行", "organization", 0, 12),
                e("植田和男", "person", 15, 27),
            ],
            expected_facts: vec![
                f("植田和男", "ceo_of", "日本銀行"),
            ],
        },
        FactsBenchmarkSample {
            id: "ja-sports-02".into(),
            language: "ja".into(),
            category: "sports".into(),
            text: "久保建英はレアル・ソシエダで活躍し、ラ・リーガのベストイレブンに選出された。".into(),
            expected_entities: vec![
                e("久保建英", "person", 0, 12),
                e("レアル・ソシエダ", "organization", 15, 36),
            ],
            expected_facts: vec![
                f("久保建英", "plays_for", "レアル・ソシエダ"),
            ],
        },
        FactsBenchmarkSample {
            id: "ja-company-02".into(),
            language: "ja".into(),
            category: "company".into(),
            text: "任天堂は京都市に本社を置くゲーム会社で、宮本茂がゲームデザイナーとして知られている。".into(),
            expected_entities: vec![
                e("任天堂", "organization", 0, 9),
                e("京都市", "location", 12, 21),
                e("宮本茂", "person", 39, 48),
            ],
            expected_facts: vec![
                f("任天堂", "located_in", "京都市"),
                f("宮本茂", "member_of", "任天堂"),
            ],
        },
        FactsBenchmarkSample {
            id: "ja-tech-02".into(),
            language: "ja".into(),
            category: "tech".into(),
            text: "ソニーグループは東京都港区に本社を置き、吉田憲一郎がCEOを務める総合エレクトロニクス企業である。".into(),
            expected_entities: vec![
                e("ソニーグループ", "organization", 0, 21),
                e("東京都港区", "location", 24, 39),
                e("吉田憲一郎", "person", 51, 66),
            ],
            expected_facts: vec![
                f("ソニーグループ", "located_in", "東京都港区"),
                f("吉田憲一郎", "ceo_of", "ソニーグループ"),
            ],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dataset_loads() {
        let samples = load_dataset();
        assert!(samples.len() >= 70, "Expected 70+ samples, got {}", samples.len());
    }

    #[test]
    fn test_seven_languages() {
        let samples = load_dataset();
        let mut langs: Vec<String> = samples.iter().map(|s| s.language.clone()).collect();
        langs.sort();
        langs.dedup();
        assert_eq!(langs.len(), 7, "Expected 7 languages, got {langs:?}");
    }

    #[test]
    fn test_each_language_has_samples() {
        let samples = load_dataset();
        for lang in &["en", "pt", "es", "de", "fr", "zh", "ja"] {
            let count = samples.iter().filter(|s| s.language == *lang).count();
            assert!(count >= 7, "Language {lang} has only {count} samples");
        }
    }

    #[test]
    fn test_entity_offsets_valid() {
        let samples = load_dataset();
        for sample in &samples {
            for entity in &sample.expected_entities {
                assert!(
                    entity.start < entity.end,
                    "Sample {}: entity '{}' has invalid offsets ({}, {})",
                    sample.id, entity.mention, entity.start, entity.end
                );
                assert!(
                    entity.end <= sample.text.len(),
                    "Sample {}: entity '{}' end {} exceeds text length {}",
                    sample.id, entity.mention, entity.end, sample.text.len()
                );
            }
        }
    }

    #[test]
    fn test_categories_present() {
        let samples = load_dataset();
        let mut cats: Vec<String> = samples.iter().map(|s| s.category.clone()).collect();
        cats.sort();
        cats.dedup();
        assert!(cats.contains(&"news".to_string()));
        assert!(cats.contains(&"wiki".to_string()));
        assert!(cats.contains(&"sports".to_string()));
        assert!(cats.contains(&"tech".to_string()));
        assert!(cats.contains(&"company".to_string()));
    }
}
