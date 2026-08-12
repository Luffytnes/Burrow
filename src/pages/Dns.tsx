import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { motion, AnimatePresence } from "framer-motion";
import { ShieldCheck, RotateCcw, Check, AlertCircle, Wifi } from "lucide-react";

// ── Types ────────────────────────────────────────────────────────────────────

interface NetworkService {
  name: string;
  dns_servers: string[];
  active: boolean;
}

type FilterType =
  "none" | "ads" | "malware" | "ads+malware" | "ads+malware+social" | "family" | "all";

interface DnsOption {
  id: string;
  label: string;
  desc: string;
  servers: string[];
  hostnames?: string[];
  doH?: string;
  filter: FilterType;
}

interface DnsProvider {
  id: string;
  name: string;
  country: string;
  flag: string;
  color: string;
  privacy: number;
  openSource: boolean | "partial";
  noLogs: boolean;
  eu: boolean;
  description: string;
  features: string[];
  options: DnsOption[];
}

// ── Providers data ────────────────────────────────────────────────────────────

const PROVIDERS: DnsProvider[] = [
  {
    id: "mullvad",
    name: "Mullvad DNS",
    country: "Suède",
    flag: "🇸🇪",
    color: "#44AD55",
    privacy: 5,
    openSource: true,
    noLogs: true,
    eu: true,
    description:
      "Géré par Mullvad VPN, pionnier suédois de la confidentialité en ligne depuis 2009. Aucune télémétrie, aucun log, aucune collecte de données personnelles. Infrastructure 100 % dédiée.",
    features: ["Sans log", "DNSSEC", "QNAME minimisation", "Anycast"],
    options: [
      {
        id: "std",
        label: "Standard",
        desc: "Résolution pure, aucun filtrage",
        servers: ["194.242.2.2"],
        hostnames: ["dns.mullvad.net"],
        doH: "https://dns.mullvad.net/dns-query",
        filter: "none",
      },
      {
        id: "adblock",
        label: "Pub & trackers",
        desc: "Bloque publicités et trackers",
        servers: ["194.242.2.3"],
        hostnames: ["adblock.dns.mullvad.net"],
        doH: "https://adblock.dns.mullvad.net/dns-query",
        filter: "ads",
      },
      {
        id: "base",
        label: "Pub + malwares",
        desc: "Bloque pub, trackers et domaines malveillants",
        servers: ["194.242.2.4"],
        hostnames: ["base.dns.mullvad.net"],
        doH: "https://base.dns.mullvad.net/dns-query",
        filter: "ads+malware",
      },
      {
        id: "extended",
        label: "Pub + malwares + social",
        desc: "Idem base + réseaux sociaux bloqués",
        servers: ["194.242.2.5"],
        hostnames: ["extended.dns.mullvad.net"],
        doH: "https://extended.dns.mullvad.net/dns-query",
        filter: "ads+malware+social",
      },
      {
        id: "family",
        label: "Famille",
        desc: "Pub, malwares, contenus adultes et jeux d'argent bloqués",
        servers: ["194.242.2.6"],
        hostnames: ["family.dns.mullvad.net"],
        doH: "https://family.dns.mullvad.net/dns-query",
        filter: "family",
      },
      {
        id: "all",
        label: "Tout filtrer",
        desc: "Filtrage maximal — pub, malwares, adulte, jeux, réseaux soc.",
        servers: ["194.242.2.9"],
        hostnames: ["all.dns.mullvad.net"],
        doH: "https://all.dns.mullvad.net/dns-query",
        filter: "all",
      },
    ],
  },
  {
    id: "quad9",
    name: "Quad9",
    country: "Suisse",
    flag: "🇨🇭",
    color: "#0099D0",
    privacy: 4,
    openSource: false,
    noLogs: true,
    eu: true,
    description:
      "Association à but non lucratif fondée à Genève. Protège contre les malwares en temps réel grâce à des flux de menaces partagés par des dizaines de partenaires en cybersécurité (IBM, NTT…). Sans publicité ni profilage.",
    features: [
      "À but non lucratif",
      "Sans log",
      "DNSSEC",
      "Protection malwares",
      "Anycast 200+ PoP",
    ],
    options: [
      {
        id: "sec",
        label: "Sécurisé",
        desc: "Blocage malwares + DNSSEC validé",
        servers: ["9.9.9.9", "149.112.112.112"],
        hostnames: ["dns.quad9.net", "dns.quad9.net"],
        doH: "https://dns.quad9.net/dns-query",
        filter: "malware",
      },
      {
        id: "unf",
        label: "Sans filtre",
        desc: "Résolution pure, DNSSEC désactivé",
        servers: ["9.9.9.10", "149.112.112.10"],
        hostnames: ["dns10.quad9.net", "dns10.quad9.net"],
        doH: "https://dns10.quad9.net/dns-query",
        filter: "none",
      },
      {
        id: "edns",
        label: "Sécurisé + ECS",
        desc: "Blocage malwares + EDNS Client Subnet",
        servers: ["9.9.9.11", "149.112.112.11"],
        hostnames: ["dns11.quad9.net", "dns11.quad9.net"],
        doH: "https://dns11.quad9.net/dns-query",
        filter: "malware",
      },
    ],
  },
  {
    id: "libredns",
    name: "LibreDNS",
    country: "Grèce",
    flag: "🇬🇷",
    color: "#5C6BC0",
    privacy: 5,
    openSource: true,
    noLogs: true,
    eu: true,
    description:
      "Service communautaire open source maintenu par des bénévoles grecs. Zéro publicité, zéro collecte, gouvernance transparente. L'une des rares alternatives gérées par la communauté plutôt que par une entreprise.",
    features: ["Open source", "Sans log", "Communautaire", "DNSSEC", "DoH disponible"],
    options: [
      {
        id: "std",
        label: "Standard",
        desc: "Résolution DNS sans filtre ni collecte",
        servers: ["116.202.176.26"],
        doH: "https://doh.libredns.gr/dns-query",
        filter: "none",
      },
    ],
  },
  {
    id: "adguard",
    name: "AdGuard DNS",
    country: "Chypre",
    flag: "🇨🇾",
    color: "#68BC71",
    privacy: 3,
    openSource: false,
    noLogs: false,
    eu: false,
    description:
      "Serveur DNS de la société AdGuard (Chypre), spécialisée dans le blocage publicitaire. Efficace contre les pubs et traqueurs, mais conserve des statistiques anonymisées. Plusieurs profils disponibles selon le niveau de filtrage souhaité.",
    features: ["Blocage pubs", "DNSSEC", "IPv6", "DoH / DoT"],
    options: [
      {
        id: "std",
        label: "Standard",
        desc: "Bloque pubs, traqueurs et domaines malveillants",
        servers: ["94.140.14.14", "94.140.15.15"],
        hostnames: ["dns.adguard-dns.com", "dns.adguard-dns.com"],
        doH: "https://dns.adguard-dns.com/dns-query",
        filter: "ads+malware",
      },
      {
        id: "family",
        label: "Famille",
        desc: "Standard + filtre contenu adulte",
        servers: ["94.140.14.15", "94.140.15.16"],
        hostnames: ["family.adguard-dns.com", "family.adguard-dns.com"],
        doH: "https://family.adguard-dns.com/dns-query",
        filter: "family",
      },
      {
        id: "unf",
        label: "Sans filtre",
        desc: "Résolution pure, aucun blocage",
        servers: ["94.140.14.140", "94.140.14.141"],
        hostnames: ["unfiltered.adguard-dns.com", "unfiltered.adguard-dns.com"],
        doH: "https://unfiltered.adguard-dns.com/dns-query",
        filter: "none",
      },
    ],
  },
  {
    id: "cloudflare",
    name: "Cloudflare",
    country: "États-Unis",
    flag: "🇺🇸",
    color: "#F48120",
    privacy: 2,
    openSource: false,
    noLogs: false,
    eu: false,
    description:
      "Service DNS ultra-rapide de Cloudflare (San Francisco). Excellent en performances (souvent classé #1 mondial), mais appartient à une entreprise américaine soumise au CLOUD Act. Auditée par KPMG, mais reste un acteur Big Tech.",
    features: ["Ultra-rapide", "DNSSEC", "Anycast mondial", "DoH / DoT"],
    options: [
      {
        id: "std",
        label: "Standard",
        desc: "DNS pur, aucun filtrage",
        servers: ["1.1.1.1", "1.0.0.1"],
        hostnames: ["one.one.one.one", "1dot1dot1dot1.cloudflare-dns.com"],
        doH: "https://cloudflare-dns.com/dns-query",
        filter: "none",
      },
      {
        id: "mal",
        label: "Anti-malware",
        desc: "Bloque les domaines malveillants connus",
        servers: ["1.1.1.2", "1.0.0.2"],
        doH: "https://security.cloudflare-dns.com/dns-query",
        filter: "malware",
      },
      {
        id: "family",
        label: "Famille",
        desc: "Filtre malwares + contenus pour adultes",
        servers: ["1.1.1.3", "1.0.0.3"],
        doH: "https://family.cloudflare-dns.com/dns-query",
        filter: "family",
      },
    ],
  },
  {
    id: "dnswatch",
    name: "DNS.WATCH",
    country: "Allemagne",
    flag: "🇩🇪",
    color: "#E53935",
    privacy: 4,
    openSource: true,
    noLogs: true,
    eu: true,
    description:
      "Service DNS minimaliste basé en Allemagne, pensé pour la neutralité et la transparence. Pas de logging, pas de filtrage, pas de censure. Idéal pour ceux qui veulent un DNS européen simple sans effets de bord.",
    features: ["Sans log", "DNSSEC", "Neutre", "Sans censure", "Open source"],
    options: [
      {
        id: "std",
        label: "Standard",
        desc: "DNS pur, neutre, aucun filtrage ni log",
        servers: ["84.200.69.80", "84.200.70.40"],
        hostnames: ["resolver1.dns.watch", "resolver2.dns.watch"],
        filter: "none",
      },
    ],
  },
];

// ── Helpers ───────────────────────────────────────────────────────────────────

// Hex bruts obligatoires : concaténés avec un alpha ("…20") plus bas
const FILTER_LABELS: Record<FilterType, { label: string; color: string }> = {
  none: { label: "Sans filtre", color: "#8b8b8b" },
  ads: { label: "Pub + trackers", color: "#e09112" },
  malware: { label: "Malwares", color: "#e05545" },
  "ads+malware": { label: "Pub + malwares", color: "#f97316" },
  "ads+malware+social": { label: "Pub + malwares + social", color: "#e0559a" },
  family: { label: "Famille", color: "#9b7fe8" },
  all: { label: "Tout filtrer", color: "#7c5cd6" },
};

function identifyDns(servers: string[]): { provider: DnsProvider; option: DnsOption } | null {
  for (const p of PROVIDERS) {
    for (const opt of p.options) {
      if (
        opt.servers.every((s) => servers.includes(s)) ||
        servers.some((s) => opt.servers.includes(s))
      ) {
        return { provider: p, option: opt };
      }
    }
  }
  return null;
}

function Stars({ n }: { n: number }) {
  return (
    <div className="flex items-center gap-0.5">
      {[1, 2, 3, 4, 5].map((i) => (
        <span
          key={i}
          style={{ fontSize: 11, color: i <= n ? "var(--warning)" : "var(--bar-track)" }}
        >
          ★
        </span>
      ))}
    </div>
  );
}

function ProviderLogo({ p }: { p: DnsProvider }) {
  const errRef = useRef(false);
  const [err, setErr] = useState(false);

  if (!err) {
    return (
      <img
        src={`/dns/${p.id}.svg`}
        alt={p.name}
        className="w-10 h-10 rounded-xl object-contain shrink-0"
        onError={() => {
          if (!errRef.current) {
            errRef.current = true;
            setErr(true);
          }
        }}
      />
    );
  }
  // Fallback letter badge
  const letters: Record<string, string> = {
    mullvad: "M",
    quad9: "9",
    libredns: "L",
    adguard: "AG",
    cloudflare: "CF",
    dnswatch: "W",
  };
  return (
    <div
      className="w-10 h-10 rounded-xl flex items-center justify-center font-bold text-[13px] shrink-0"
      style={{ background: p.color + "20", color: p.color, border: `1px solid ${p.color}30` }}
    >
      {letters[p.id] ?? p.name[0]}
    </div>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

export default function DnsPage() {
  const [services, setServices] = useState<NetworkService[]>([]);
  const [selectedSvc, setSelectedSvc] = useState("");
  const [loading, setLoading] = useState(true);
  const [applying, setApplying] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; msg: string } | null>(null);

  // Per-provider selected option
  const [selections, setSelections] = useState<Record<string, string>>(() =>
    Object.fromEntries(PROVIDERS.map((p) => [p.id, p.options[0].id]))
  );
  const [dnsMode, setDnsMode] = useState<"hostname" | "ip">("hostname");
  const [useDoH, setUseDoH] = useState(true);
  const [installingDoH, setInstallingDoH] = useState(false);
  const [searchDomains, setSearchDomains] = useState<string[]>([]);
  const [newDomain, setNewDomain] = useState("");
  const [savingDomains, setSavingDomains] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const svcs = await invoke<NetworkService[]>("list_network_services");
      setServices(svcs);
      setSelectedSvc((prev) => {
        if (!prev && svcs.length > 0) {
          return (svcs.find((s) => s.active) ?? svcs[0]).name;
        }
        return prev;
      });
    } catch (e) {
      console.error("load dns services:", e);
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  useEffect(() => {
    if (!selectedSvc) return;
    invoke<string[]>("get_search_domains", { service: selectedSvc })
      .then(setSearchDomains)
      .catch((e) => console.error("get_search_domains:", e));
  }, [selectedSvc]);

  const currentSvc = services.find((s) => s.name === selectedSvc);
  const currentDns = currentSvc?.dns_servers ?? [];
  const identified = identifyDns(currentDns);

  const flash = (ok: boolean, msg: string) => {
    setResult({ ok, msg });
    setTimeout(() => setResult(null), 3500);
  };

  const saveSearchDomains = async (domains: string[]) => {
    if (!selectedSvc) return;
    setSavingDomains(true);
    try {
      await invoke("set_search_domains", { service: selectedSvc, domains });
    } catch (e) {
      console.error("set_search_domains:", e);
    }
    setSavingDomains(false);
  };

  const addDomain = () => {
    const d = newDomain.trim().toLowerCase();
    if (!d || searchDomains.includes(d)) {
      setNewDomain("");
      return;
    }
    const next = [...searchDomains, d];
    setSearchDomains(next);
    setNewDomain("");
    saveSearchDomains(next);
  };

  const removeDomain = (d: string) => {
    const next = searchDomains.filter((x) => x !== d);
    setSearchDomains(next);
    saveSearchDomains(next);
  };

  const installDoH = async (provider: DnsProvider) => {
    const optId = selections[provider.id];
    const option = provider.options.find((o) => o.id === optId);
    if (!option?.doH) return;
    setInstallingDoH(true);
    try {
      await invoke("install_doh_profile", {
        providerId: provider.id,
        optionId: option.id,
      });
      flash(true, "Profil DoH prêt — validez l'installation dans les Réglages Système");
    } catch (e) {
      flash(false, String(e));
    }
    setInstallingDoH(false);
  };

  const getServers = (option: DnsOption) =>
    dnsMode === "hostname" && option.hostnames?.length ? option.hostnames : option.servers;

  const apply = async (provider: DnsProvider) => {
    if (!selectedSvc) return;
    const optId = selections[provider.id];
    const option = provider.options.find((o) => o.id === optId);
    if (!option) return;
    setApplying(true);
    try {
      await invoke("set_dns_servers", { service: selectedSvc, servers: getServers(option) });
      flash(true, `DNS ${provider.name} — ${option.label} appliqué avec succès`);
      await load();
    } catch (e) {
      flash(false, String(e));
    }
    setApplying(false);
  };

  const reset = async () => {
    if (!selectedSvc) return;
    setResetting(true);
    try {
      await invoke("reset_dns", { service: selectedSvc });
      flash(true, "DNS réinitialisé — votre routeur gère désormais la résolution");
      await load();
    } catch (e) {
      flash(false, String(e));
    }
    setResetting(false);
  };

  return (
    <div className="flex flex-col h-full overflow-hidden" style={{ position: "relative" }}>
      {/* ── Header ── */}
      <div className="px-6 pt-4 pb-3 shrink-0">
        <div className="flex items-start justify-between gap-4">
          <div className="flex items-center gap-3">
            <div
              className="w-9 h-9 rounded-xl flex items-center justify-center shrink-0"
              style={{ background: "var(--bg-card)", border: "1px solid var(--border)" }}
            >
              <ShieldCheck size={16} style={{ color: "var(--accent)" }} />
            </div>
            <div>
              <h2 className="text-base font-bold" style={{ color: "var(--text-1)" }}>
                DNS Privé
              </h2>
              <p className="text-[11px]" style={{ color: "var(--text-3)" }}>
                Remplacez les DNS de votre FAI pour protéger votre navigation
              </p>
            </div>
          </div>

          {/* Interface selector + current DNS + reset */}
          <div className="flex items-center gap-2 shrink-0">
            {/* Active interface badge */}
            <div
              className="flex items-center gap-1.5 text-[11px] font-medium px-3 py-1.5 rounded-lg"
              style={{
                background: "var(--bg-card)",
                border: "1px solid var(--border)",
                color: "var(--text-1)",
              }}
            >
              <Wifi size={11} style={{ color: "var(--accent)" }} />
              <span>{selectedSvc || "—"}</span>
            </div>

            {/* Current DNS badge */}
            {currentDns.length > 0 ? (
              <div
                className="text-[10px] px-2.5 py-1.5 rounded-lg"
                style={{
                  background: identified ? identified.provider.color + "15" : "var(--bg-card)",
                  border: `1px solid ${identified ? identified.provider.color + "40" : "var(--border)"}`,
                  color: identified ? identified.provider.color : "var(--text-3)",
                }}
              >
                {identified
                  ? `${identified.provider.name} — ${identified.option.label}`
                  : currentDns[0]}
              </div>
            ) : (
              <div
                className="text-[10px] px-2.5 py-1.5 rounded-lg"
                style={{
                  background: "var(--bg-card)",
                  border: "1px solid var(--border)",
                  color: "var(--text-3)",
                }}
              >
                DNS automatique (FAI)
              </div>
            )}

            {/* Mode toggle: Classique / DoH */}
            <div
              className="flex items-center rounded-lg overflow-hidden"
              style={{ border: "1px solid var(--border)", background: "var(--bg-card)" }}
            >
              <button
                onClick={() => setUseDoH(false)}
                className="text-[10px] font-semibold px-2.5 py-1.5 transition-all"
                style={{
                  background: !useDoH ? "var(--accent)" : "transparent",
                  color: !useDoH ? "#fff" : "var(--text-3)",
                }}
              >
                Classique
              </button>
              <button
                onClick={() => setUseDoH(true)}
                className="text-[10px] font-semibold px-2.5 py-1.5 transition-all"
                style={{
                  background: useDoH ? "var(--violet)" : "transparent",
                  color: useDoH ? "#fff" : "var(--text-3)",
                }}
              >
                DoH
              </button>
            </div>

            {/* Hostname / IP (Classique only) */}
            {!useDoH && (
              <div
                className="flex items-center rounded-lg overflow-hidden"
                style={{ border: "1px solid var(--border)", background: "var(--bg-card)" }}
              >
                {(["hostname", "ip"] as const).map((mode) => (
                  <button
                    key={mode}
                    onClick={() => setDnsMode(mode)}
                    className="text-[10px] font-semibold px-2.5 py-1.5 transition-all"
                    style={{
                      background: dnsMode === mode ? "var(--accent)" : "transparent",
                      color: dnsMode === mode ? "#fff" : "var(--text-3)",
                    }}
                  >
                    {mode === "hostname" ? "Hostname" : "IP"}
                  </button>
                ))}
              </div>
            )}

            {/* Reset button */}
            <button
              onClick={reset}
              disabled={resetting || applying}
              className="flex items-center gap-1.5 text-[11px] font-medium px-3 py-1.5 rounded-lg transition-opacity"
              style={{
                background: "var(--bg-card)",
                border: "1px solid var(--border)",
                color: "var(--text-2)",
                opacity: resetting ? 0.5 : 1,
              }}
            >
              <RotateCcw size={11} className={resetting ? "animate-spin" : ""} />
              Réinitialiser
            </button>
          </div>
        </div>

        {/* Search domains */}
        <div className="mt-3 flex items-start gap-2">
          <span
            className="text-[10px] font-semibold shrink-0 mt-1.5"
            style={{ color: "var(--text-3)", minWidth: 130 }}
          >
            Domaines de recherche
          </span>
          <div className="flex-1 flex flex-wrap items-center gap-1.5">
            {searchDomains.map((d) => (
              <div
                key={d}
                className="flex items-center gap-1 pl-2 pr-1 py-0.5 rounded-full text-[10px] font-medium"
                style={{
                  background: "var(--bg-card)",
                  border: "1px solid var(--border)",
                  color: "var(--text-2)",
                }}
              >
                {d}
                <button
                  onClick={() => removeDomain(d)}
                  className="opacity-50 hover:opacity-100 transition-opacity ml-0.5"
                  style={{ color: "var(--text-3)" }}
                >
                  <span style={{ fontSize: 10, lineHeight: 1 }}>✕</span>
                </button>
              </div>
            ))}
            <form
              onSubmit={(e) => {
                e.preventDefault();
                addDomain();
              }}
              className="flex items-center"
            >
              <input
                value={newDomain}
                onChange={(e) => setNewDomain(e.target.value)}
                placeholder="Ajouter un domaine…"
                className="text-[10px] px-2 py-0.5 rounded-full outline-none"
                style={{
                  background: "transparent",
                  border: "1px dashed var(--border)",
                  color: "var(--text-1)",
                  width: newDomain ? `${Math.max(120, newDomain.length * 7)}px` : 120,
                }}
                onBlur={addDomain}
                disabled={savingDomains}
              />
            </form>
          </div>
        </div>

        {/* Toast notification */}
        <AnimatePresence>
          {result && (
            <motion.div
              initial={{ opacity: 0, y: -6 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -6 }}
              className="mt-2 flex items-center gap-2 text-[11px] px-3 py-2 rounded-lg"
              style={{
                background: result.ok ? "var(--success-dim)" : "var(--danger-dim)",
                border: `1px solid ${result.ok ? "var(--success-soft)" : "var(--danger-soft)"}`,
                color: result.ok ? "var(--success)" : "var(--danger)",
              }}
            >
              {result.ok ? <Check size={12} /> : <AlertCircle size={12} />}
              {result.msg}
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      {/* ── Provider grid ── */}
      <div className="flex-1 overflow-y-auto px-6 pb-4">
        {loading ? (
          <div className="flex items-center justify-center h-40">
            <div
              className="w-5 h-5 rounded-full border-2 border-t-transparent animate-spin"
              style={{ borderColor: "var(--accent)", borderTopColor: "transparent" }}
            />
          </div>
        ) : (
          <div className="grid grid-cols-2 gap-3">
            {PROVIDERS.map((provider) => {
              const selOptId = selections[provider.id];
              const isActive = identified?.provider.id === provider.id;

              return (
                <motion.div
                  key={provider.id}
                  initial={{ opacity: 0, y: 8 }}
                  animate={{ opacity: 1, y: 0 }}
                  className="card p-4 flex flex-col gap-3"
                  style={{
                    borderColor: isActive ? provider.color + "60" : undefined,
                    boxShadow: isActive ? `0 0 0 1px ${provider.color}30` : undefined,
                  }}
                >
                  {/* Card header */}
                  <div className="flex items-start gap-3">
                    <ProviderLogo p={provider} />
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2 flex-wrap">
                        <span className="font-bold text-[13px]" style={{ color: "var(--text-1)" }}>
                          {provider.name}
                        </span>
                        {isActive && (
                          <span
                            className="text-[9px] font-semibold px-1.5 py-0.5 rounded-full"
                            style={{ background: provider.color + "20", color: provider.color }}
                          >
                            Actif
                          </span>
                        )}
                      </div>
                      <div className="flex items-center gap-2 mt-0.5 flex-wrap">
                        <span className="text-[11px]" style={{ color: "var(--text-3)" }}>
                          {provider.flag} {provider.country}
                        </span>
                        <Stars n={provider.privacy} />
                        {provider.eu && (
                          <span
                            className="text-[9px] font-semibold px-1 py-0.5 rounded"
                            style={{ background: "#003399" + "20", color: "#003399" }}
                          >
                            🇪🇺 Europe
                          </span>
                        )}
                      </div>
                    </div>
                  </div>

                  {/* Description */}
                  <p className="text-[11px] leading-relaxed" style={{ color: "var(--text-2)" }}>
                    {provider.description}
                  </p>

                  {/* Feature chips */}
                  <div className="flex flex-wrap gap-1">
                    {provider.features.map((f) => (
                      <span
                        key={f}
                        className="text-[9px] font-medium px-2 py-0.5 rounded-full"
                        style={{ background: "var(--bar-track)", color: "var(--text-3)" }}
                      >
                        {f}
                      </span>
                    ))}
                    <span
                      className="text-[9px] font-medium px-2 py-0.5 rounded-full"
                      style={{ background: "var(--bar-track)", color: "var(--text-3)" }}
                    >
                      {provider.noLogs ? "✓ Sans log" : "Logs anonymes"}
                    </span>
                    <span
                      className="text-[9px] font-medium px-2 py-0.5 rounded-full"
                      style={{ background: "var(--bar-track)", color: "var(--text-3)" }}
                    >
                      {provider.openSource === true
                        ? "✓ Open source"
                        : provider.openSource === "partial"
                          ? "Partiel OS"
                          : "Propriétaire"}
                    </span>
                  </div>

                  {/* Options — dropdown */}
                  {(() => {
                    const selOpt =
                      provider.options.find((o) => o.id === selOptId) ?? provider.options[0];
                    return (
                      <div className="flex flex-col gap-2">
                        <select
                          value={selOptId}
                          onChange={(e) =>
                            setSelections((s) => ({ ...s, [provider.id]: e.target.value }))
                          }
                          className="w-full text-[11px] font-medium px-2.5 py-1.5 rounded-lg outline-none appearance-none"
                          style={{
                            background: "var(--bg)",
                            border: `1px solid ${provider.color}50`,
                            color: "var(--text-1)",
                            cursor: "pointer",
                          }}
                        >
                          {provider.options.map((opt) => (
                            <option key={opt.id} value={opt.id}>
                              {opt.label} — {FILTER_LABELS[opt.filter].label}
                            </option>
                          ))}
                        </select>
                        {/* Selected option detail */}
                        <div
                          className="px-2.5 py-2 rounded-lg"
                          style={{ background: "var(--bg)", border: `1px solid var(--border)` }}
                        >
                          <p className="text-[10px]" style={{ color: "var(--text-2)" }}>
                            {selOpt.desc}
                          </p>
                          <p
                            className="font-mono text-[9px] mt-1"
                            style={{ color: "var(--text-3)" }}
                          >
                            {getServers(selOpt).join(" · ")}
                          </p>
                        </div>
                      </div>
                    );
                  })()}

                  {/* Apply / DoH button */}
                  {useDoH ? (
                    (() => {
                      const opt = provider.options.find((o) => o.id === selOptId);
                      const hasDoH = !!opt?.doH;
                      return (
                        <button
                          onClick={() => installDoH(provider)}
                          disabled={installingDoH || !hasDoH}
                          className="mt-auto w-full py-2 rounded-xl text-[12px] font-semibold transition-all flex items-center justify-center gap-1.5"
                          style={{
                            background: hasDoH ? "var(--violet)" : "var(--bar-track)",
                            color: hasDoH ? "#fff" : "var(--text-3)",
                            opacity: installingDoH ? 0.6 : 1,
                            cursor: hasDoH ? "pointer" : "default",
                          }}
                        >
                          {installingDoH
                            ? "Préparation…"
                            : hasDoH
                              ? "🔒 Installer le profil DoH"
                              : "DoH non disponible"}
                        </button>
                      );
                    })()
                  ) : (
                    <button
                      onClick={() => apply(provider)}
                      disabled={applying || resetting || !selectedSvc}
                      className="mt-auto w-full py-2 rounded-xl text-[12px] font-semibold transition-all"
                      style={{
                        background:
                          isActive && identified?.option.id === selOptId
                            ? "var(--bar-track)"
                            : provider.color,
                        color:
                          isActive && identified?.option.id === selOptId ? "var(--text-3)" : "#fff",
                        opacity: applying || resetting ? 0.5 : 1,
                        cursor:
                          isActive && identified?.option.id === selOptId ? "default" : "pointer",
                      }}
                    >
                      {isActive && identified?.option.id === selOptId
                        ? "✓ Déjà actif"
                        : applying
                          ? "Application…"
                          : "Appliquer"}
                    </button>
                  )}
                </motion.div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
