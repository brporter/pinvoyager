import { LegalPage } from "./LegalPage";

/** Privacy disclosures matching current storage, publication, and session behavior. */
export function Privacy() {
  return (
    <LegalPage title="Privacy Policy">
      <section>
        <h2>1. Information we handle</h2>
        <p>
          When you sign in through Google, Microsoft, or Apple, we receive identity information from
          that provider. We store the provider identifier linked to your account, profile
          information such as your chosen display name and any saved avatar URL, account and
          identity timestamps, and session records. We do not store your provider password or
          long-lived provider access or refresh tokens.
        </p>
        <p>
          We store your voyage titles, descriptions, publication and location-precision choices,
          uploaded photos, original uploads, generated image variants, and photo metadata. Metadata
          can include exact coordinates, capture times, dimensions, and camera details extracted
          from EXIF. Removing metadata from displayed images does not remove the exact metadata
          stored by the service.
        </p>
        <p>
          Operating the service also involves request information such as IP addresses, browser
          headers, request times, errors, and security events at the application or hosting layer.
          Optional diagnostic tracing records request and processing activity. If you contact us, we
          handle the information you send.
        </p>
      </section>
      <section>
        <h2>2. Why we use information</h2>
        <p>
          We use information to sign you in, maintain accounts and sessions, process and display
          photos and maps, apply privacy settings, answer requests, investigate abuse, protect the
          service, and meet legal obligations. Where applicable law requires a legal basis, these
          activities rely on providing the requested service, legitimate interests in security and
          operation, legal obligations, or consent where required. You may withdraw consent for
          processing based on consent by contacting us.
        </p>
      </section>
      <section>
        <h2>3. Public voyages, drafts, and location</h2>
        <p>
          Published voyages are public. Their photos, voyage text, display name, and map locations
          can be viewed and copied by others. Public map coordinates follow the voyage’s chosen
          precision; selecting exact precision exposes exact coordinates. Rounded locations and
          visible landmarks can still reveal where a photo was taken.
        </p>
        <p>
          Draft voyages and their photos are restricted to the owner through normal service access.
          Administrators can view draft titles and descriptions for account moderation; that
          interface does not grant access to private photos. Authorized operators and infrastructure
          providers may access stored data as needed for operation, security, or legal obligations.
          Owner-only photo metadata includes exact coordinates. We serve re-encoded images with
          embedded metadata stripped, never original uploads.
        </p>
      </section>
      <section>
        <h2>4. Cookies and other providers</h2>
        <p>
          We use cookies for sign-in security and server-side sessions. Blocking these cookies
          prevents sign-in from working. Signing out revokes the current session. The application
          does not currently include advertising or third-party analytics trackers.
        </p>
        <p>
          Sign-in providers receive information when you authenticate with them. Your browser also
          requests map tiles directly from OpenStreetMap’s tile service, which receives your IP
          address, browser request information, and the requested tile area. Tiles can reveal the
          area you are viewing. See the{" "}
          <a href="https://osmfoundation.org/wiki/Privacy_Policy">
            OpenStreetMap Foundation privacy policy
          </a>{" "}
          and your chosen sign-in provider’s policy. Hosting and infrastructure providers process
          data needed to run the service.
        </p>
      </section>
      <section>
        <h2>5. Sharing and legal disclosures</h2>
        <p>
          We do not sell personal information or share it for targeted advertising. We share
          information with providers as needed to operate the service, when you direct us to publish
          content, and as legally permitted or required to protect people and rights, investigate
          abuse, or respond to lawful requests. Content we deem illegal will be referred to
          appropriate authorities; relevant evidence and account information may be preserved or
          disclosed as permitted or required by law.
        </p>
      </section>
      <section>
        <h2>6. Retention, deletion, and security</h2>
        <p>
          We retain account data and uploads while needed to provide the service and for security,
          dispute resolution, and legal obligations. Deleting a photo or voyage removes its database
          records and queues unused files for background cleanup; this does not erase every stored
          copy immediately. Shared files can remain while another upload references them. Backups,
          logs, and legally preserved records can outlast deletion. We cannot remove copies other
          people have saved.
        </p>
        <p>
          Retention periods for hosting logs, diagnostics, and backups: [operator to confirm before
          this policy takes effect]. We use access controls and session protections, but no storage
          or transmission method guarantees complete security. Hosting locations and international
          transfer safeguards: [operator to confirm].
        </p>
      </section>
      <section>
        <h2>7. Your choices and requests</h2>
        <p>
          You can edit your display name, unpublish voyages, change location precision, and delete
          photos or voyages using the service. Previous display names remain reserved or available
          as aliases to protect account routing. There is no self-service account deletion at
          present; request account deletion at{" "}
          <a href="mailto:inquiries@bryanporter.com">inquiries@bryanporter.com</a>. We may need to
          verify that you own the account before acting.
        </p>
        <p>
          Depending on your location, you may have rights to access, correct, delete, or obtain a
          copy of personal information, restrict or object to processing, and complain to your data
          protection authority. Contact{" "}
          <a href="mailto:inquiries@bryanporter.com">inquiries@bryanporter.com</a> to exercise these
          rights. Legal exceptions and identity-verification requirements may apply.
        </p>
      </section>
      <section>
        <h2>8. Children, updates, and contact</h2>
        <p>
          Accounts and uploads are intended for adults aged 18 or older. We do not knowingly collect
          personal information from children. Contact us if you believe a child has supplied
          information so we can investigate and take appropriate action. We will update the date
          when this policy changes and provide additional notice where required by law. Questions:
          <a href="mailto:inquiries@bryanporter.com">inquiries@bryanporter.com</a>. Effective date:
          [to be confirmed after review].
        </p>
      </section>
    </LegalPage>
  );
}
