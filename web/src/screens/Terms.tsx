import { Link } from "react-router";
import { LegalPage } from "./LegalPage";

/** Terms for the photo-hosting service, pending legal review and an effective date. */
export function Terms() {
  return (
    <LegalPage title="Terms of Service">
      <section>
        <h2>1. Using PinVoyager</h2>
        <p>
          These terms govern your use of PinVoyager, the photo and voyage mapping service operated
          by the operator identified above ("we", "us"). By using the service after these terms take
          effect, you agree to them. If you disagree, stop using the service. You must be at least
          18 and able to enter a binding agreement to create an account or upload content. Keep your
          sign-in provider account secure and tell us about suspected unauthorized use. Our{" "}
          <Link to="/privacy">Privacy Policy</Link> explains data handling.
        </p>
      </section>
      <section>
        <h2>2. Your content and permissions</h2>
        <p>
          You retain ownership of your photos, voyage descriptions, and other submissions ("user
          content"). You are responsible for their legality, accuracy, and consequences. Upload only
          material you own or have permission to use, including permission needed from people
          pictured and for any personal information or locations you disclose.
        </p>
        <p>
          You grant us a nonexclusive, worldwide, royalty-free license to store, copy, process,
          re-encode, and display your content to operate and protect the service, and to let our
          hosting providers do that work for us. Public display follows your publication choices.
          This license ends when content is removed from the service, except for copies reasonably
          needed for backups, legal compliance, or resolving disputes. It does not transfer
          ownership.
        </p>
        <p>
          Publishing makes your voyage and its photos public. Location precision follows your chosen
          setting, including exact locations if selected. Others may save or redistribute public
          material; unpublishing or deletion cannot recall their copies. Keep your own backups.
        </p>
      </section>
      <section>
        <h2>3. Prohibited content and conduct</h2>
        <p>
          Do not upload illegal material, child sexual exploitation material, nonconsensual intimate
          images, threats, harassment, hateful abuse, private information disclosed without
          permission, fraudulent material, or content that infringes others’ rights. Do not
          impersonate people, distribute malware or spam, evade suspensions, bypass security or
          upload limits, or disrupt the service. This list does not limit our discretion to remove
          other content we consider objectionable.
        </p>
      </section>
      <section>
        <h2>4. Removal, suspension, and reporting to authorities</h2>
        <p>
          <strong>
            We reserve the right to refuse, restrict, unpublish, or remove any user content at any
            time, for any reason, at our sole discretion. Content we deem objectionable in our sole
            discretion will be removed without warning or prior notice.
          </strong>{" "}
          We may also suspend or terminate accounts or access to the service. We need not provide
          advance notice or compensation, except where applicable law requires otherwise.
        </p>
        <p>
          <strong>
            Any content we deem illegal will be referred to the appropriate authorities.
          </strong>{" "}
          We may preserve relevant evidence and disclose content and related account information as
          permitted or required by law. A referral reports suspected illegality; it is not a court
          determination. These terms do not promise continuous monitoring or review of every upload.
          Mandatory notices, reasons, and review rights under applicable law remain available.
        </p>
      </section>
      <section>
        <h2>5. Hosted content and third parties</h2>
        <p>
          To the fullest extent permitted by law, we are not responsible for user content or any
          harm arising from it. Hosting does not mean that we endorse, verify, or guarantee its
          accuracy, safety, legality, or suitability. You view and rely on user content at your own
          risk. Maps and photo locations are informational and may be inaccurate; do not rely on
          them for navigation or safety. Sign-in and map providers operate under their own terms and
          privacy policies.
        </p>
      </section>
      <section>
        <h2>6. Content and copyright complaints</h2>
        <p>
          Send complaints to{" "}
          <a href="mailto:inquiries@bryanporter.com">inquiries@bryanporter.com</a> with the specific
          voyage or photo URL, the reason for the complaint, and your contact details. For copyright
          complaints, identify the protected work, the allegedly infringing material, and your
          authority to act for the rights holder. We may remove disputed content and terminate
          repeat infringers in appropriate circumstances. Any legally required notice,
          counter-notice, or appeal process applies. Do not send illegal images as attachments.
        </p>
      </section>
      <section>
        <h2>7. Service availability and liability</h2>
        <p>
          We provide the service as available, without guarantees of uninterrupted access, permanent
          storage, or freedom from errors. To the fullest extent permitted by law, we disclaim
          implied warranties of merchantability, fitness for a particular purpose, and
          noninfringement, and exclude liability for indirect or consequential losses, including
          lost data or profits arising from use of the service or hosted content. Nothing excludes
          liability or consumer rights that the law does not allow us to exclude.
        </p>
      </section>
      <section>
        <h2>8. Changes, disputes, and contact</h2>
        <p>
          We may change or discontinue the service. Revised terms will show an updated date;
          material changes take effect 30 days after posting unless a different period is required
          by law or an urgent legal or security change is necessary. We will provide additional
          notice when legally required. Continued use after the effective date constitutes
          acceptance to the extent permitted by law.
        </p>
        <p>
          Missouri law and applicable United States federal law govern these terms. Disputes may be
          brought in the state or federal courts with jurisdiction in Missouri. Mandatory local
          consumer protections remain applicable. If a provision is unenforceable, the remaining
          terms continue to apply. Questions and complaints:{" "}
          <a href="mailto:inquiries@bryanporter.com">inquiries@bryanporter.com</a>. Effective date:
          [to be confirmed after review].
        </p>
      </section>
    </LegalPage>
  );
}
