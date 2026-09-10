import type { Locale } from "../../i18n";

export function LanguageFlag({ locale }: { locale: Locale }) {
  return (
    <svg
      aria-hidden="true"
      className="h-5 w-5 shrink-0"
      viewBox="0 0 20 20"
      fill="none"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {locale === "zh" ? (
        <>
          <circle cx="10" cy="10" r="8.25" fill="#de2910" />
          <path
            d="m7 6 .55 1.3 1.4.12-1.06.9.3 1.38-1.2-.72-1.2.72.3-1.38-1.06-.9 1.4-.12Z"
            fill="#ffde00"
            stroke="none"
          />
          <circle cx="11.2" cy="6.3" r=".45" fill="#ffde00" />
          <circle cx="12.3" cy="7.4" r=".45" fill="#ffde00" />
          <circle cx="12.1" cy="8.8" r=".45" fill="#ffde00" />
          <circle cx="10.9" cy="9.6" r=".45" fill="#ffde00" />
        </>
      ) : (
        <>
          <circle cx="10" cy="10" r="8.25" fill="#1f4b99" />
          <path d="m4.7 5.4 10.6 9.2M15.3 5.4 4.7 14.6" stroke="#fff" strokeWidth="2.7" />
          <path d="M10 3.2v13.6M3.2 10h13.6" stroke="#fff" strokeWidth="3" />
          <path d="m4.7 5.4 10.6 9.2M15.3 5.4 4.7 14.6" stroke="#cf142b" strokeWidth="1" />
          <path d="M10 3.2v13.6M3.2 10h13.6" stroke="#cf142b" strokeWidth="1.35" />
        </>
      )}
      <circle cx="10" cy="10" r="8.25" fill="none" stroke="currentColor" strokeOpacity=".28" />
    </svg>
  );
}

export default LanguageFlag;
