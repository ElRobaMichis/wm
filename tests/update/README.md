# Clave de prueba del actualizador

`test-key.pem` es una clave ECDSA P-256 **pública a propósito**: firma los
feeds falsos de las pruebas (`wm ci-probe`, `sign.rs`). Un binario solo la
acepta si se compiló con `WM_UPDATE_PUBKEY` igual a su clave pública
(`8fd2ee4c…4408`); el de los releases no. La clave real de los releases no
está en el repositorio: vive en el secret `WM_UPDATE_KEY` de GitHub
(`plan-update.md` U0).
